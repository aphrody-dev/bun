//! ES module loading: Bun-style resolution (extension and `index` probing, `node_modules`, `package.json`),
//! every module transpiled by Bun's transpiler before Boa parses it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use boa_engine::module::{ModuleLoader, ModuleRequest, Referrer};
use boa_engine::{Context, JsError, JsNativeError, JsObject, JsResult, JsString, Module, Source, js_string};

const EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "json"];

#[derive(Default)]
pub struct BunModuleLoader {
    modules: RefCell<HashMap<PathBuf, Module>>,
}

fn type_error(message: String) -> JsError {
    JsNativeError::typ().with_message(message).into()
}

fn file(path: &Path) -> Option<PathBuf> {
    path.is_file().then(|| path.to_path_buf())
}

fn with_extension(path: &Path) -> Option<PathBuf> {
    file(path).or_else(|| {
        EXTENSIONS.iter().find_map(|ext| {
            let mut candidate = path.as_os_str().to_owned();
            candidate.push(".");
            candidate.push(ext);
            file(Path::new(&candidate))
        })
    })
}

fn package_entry(dir: &Path) -> Option<PathBuf> {
    let json = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&json).ok()?;
    let exports = manifest.get("exports").and_then(|e| match e {
        serde_json::Value::String(s) => Some(s.as_str()),
        serde_json::Value::Object(map) => {
            let root = map.get(".").unwrap_or(e);
            root.as_str().or_else(|| {
                ["bun", "import", "default"]
                    .iter()
                    .find_map(|k| root.get(k).and_then(serde_json::Value::as_str))
            })
        }
        _ => None,
    });
    let entry = exports
        .or_else(|| manifest.get("module").and_then(serde_json::Value::as_str))
        .or_else(|| manifest.get("main").and_then(serde_json::Value::as_str))?;
    with_extension(&dir.join(entry))
}

fn as_file_or_dir(path: &Path) -> Option<PathBuf> {
    with_extension(path)
        .or_else(|| path.is_dir().then(|| package_entry(path)).flatten())
        .or_else(|| EXTENSIONS.iter().find_map(|ext| file(&path.join(format!("index.{ext}")))))
}

pub fn resolve(specifier: &str, from_dir: &Path) -> Option<PathBuf> {
    let specifier = specifier.strip_prefix("file://").unwrap_or(specifier);
    if specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../") || specifier == "." || specifier == ".." {
        return as_file_or_dir(&from_dir.join(specifier)).and_then(|p| p.canonicalize().ok());
    }
    from_dir
        .ancestors()
        .find_map(|dir| as_file_or_dir(&dir.join("node_modules").join(specifier)))
        .and_then(|p| p.canonicalize().ok())
}

impl BunModuleLoader {
    pub fn load(&self, path: &Path, context: &mut Context) -> JsResult<Module> {
        if let Some(module) = self.modules.borrow().get(path) {
            return Ok(module.clone());
        }
        let contents = std::fs::read(path).map_err(|err| type_error(format!("could not read {}: {err}", path.display())))?;
        let module = if path.extension().is_some_and(|ext| ext == "json") {
            let text = String::from_utf8_lossy(&contents);
            Module::parse_json(JsString::from(text.as_ref()), context)?
        } else {
            let js = crate::transpile::transpile(path.as_os_str().as_encoded_bytes(), &contents)
                .map_err(|err| JsNativeError::syntax().with_message(err))?;
            Module::parse(Source::from_bytes(&js).with_path(path), None, context)?
        };
        self.modules.borrow_mut().insert(path.to_path_buf(), module.clone());
        Ok(module)
    }
}

impl ModuleLoader for BunModuleLoader {
    fn load_imported_module(
        self: Rc<Self>,
        referrer: Referrer,
        request: ModuleRequest,
        context: &RefCell<&mut Context>,
    ) -> impl Future<Output = JsResult<Module>> {
        let specifier = request.specifier().to_std_string_escaped();
        let from_dir = referrer
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let result = match resolve(&specifier, &from_dir) {
            Some(path) => self.load(&path, &mut context.borrow_mut()),
            None => Err(type_error(format!("Cannot find module '{specifier}' from '{}'", from_dir.display()))),
        };
        std::future::ready(result)
    }

    fn init_import_meta(self: Rc<Self>, import_meta: &JsObject, module: &Module, context: &mut Context) {
        let Some(path) = module.path() else { return };
        let dir = path.parent().unwrap_or(path);
        let name = path.file_name().unwrap_or_default();
        let set = |key: JsString, value: String, context: &mut Context| {
            let _ = import_meta.set(key, JsString::from(value.as_str()), false, context);
        };
        set(js_string!("url"), format!("file://{}", path.display()), context);
        set(js_string!("path"), path.display().to_string(), context);
        set(js_string!("filename"), path.display().to_string(), context);
        set(js_string!("file"), name.to_string_lossy().into_owned(), context);
        set(js_string!("dir"), dir.display().to_string(), context);
        set(js_string!("dirname"), dir.display().to_string(), context);
        let main = crate::runtime::main_path().is_some_and(|main| main == path);
        let _ = import_meta.set(js_string!("main"), main, false, context);
    }
}

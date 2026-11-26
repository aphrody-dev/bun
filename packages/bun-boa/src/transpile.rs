//! TypeScript/JSX → JavaScript with Bun's parser and printer, the way `bun run` transpiles a file.

use bun_alloc::Arena;
use bun_ast::Loader;

pub fn loader_for(path: &[u8]) -> Loader {
    let ext = path.rsplit(|&b| b == b'.').next().unwrap_or_default();
    match ext {
        b"ts" | b"mts" | b"cts" => Loader::Ts,
        b"tsx" => Loader::Tsx,
        _ => Loader::Jsx,
    }
}

/// The parser allocates AST nodes from thread-local stores; they must outlive the print.
struct StoreScope(bun_ast::StoreAstAllocHeap);

impl StoreScope {
    fn enter() -> StoreScope {
        bun_ast::expr::data::Store::create();
        bun_ast::stmt::data::Store::create();
        StoreScope(bun_ast::StoreAstAllocHeap::new())
    }
}

impl Drop for StoreScope {
    fn drop(&mut self) {
        bun_ast::expr::data::Store::deinit();
        bun_ast::stmt::data::Store::deinit();
    }
}

fn messages(log: &bun_ast::Log) -> String {
    let mut out = String::new();
    let _ = log.print(&mut out);
    out
}

pub fn transpile(path: &[u8], contents: &[u8]) -> Result<Vec<u8>, String> {
    let loader = loader_for(path);
    let _store = StoreScope::enter();
    let arena = Arena::new();
    let source = bun_ast::Source::init_path_string(path, contents);

    let mut opts = bun_js_parser::ParserOptions::init(Default::default(), loader);
    opts.features.top_level_await = true;
    opts.features.standard_decorators = !loader.is_typescript();
    opts.transform_only = true;
    let define: &bun_js_parser::Define = arena.alloc(bun_js_parser::Define::default());
    let mut log = bun_ast::Log::init();
    let parser = bun_js_parser::Parser::init(opts, &mut log, &source, define, &arena)
        .map_err(|_| messages(&log))?;
    let mut ast = match parser.parse() {
        Ok(bun_js_parser::Result::Ast(ast)) if log.errors == 0 => ast,
        _ => return Err(messages(&log)),
    };

    let alloc = *ast.symbols.allocator();
    let list = core::mem::replace(&mut ast.symbols, bun_alloc::ArenaVec::new_in(alloc));
    let symbols = bun_ast::symbol::Map::init_with_one_list(list.into_iter().collect());
    let mut printer = bun_js_printer::BufferPrinter::init(bun_js_printer::BufferWriter::init());
    let options = bun_js_printer::Options {
        require_ref: Some(ast.require_ref),
        import_meta_ref: ast.import_meta_ref,
        ..Default::default()
    };
    bun_js_printer::print_ast::<false, false>(&mut printer, &arena, &ast, symbols, &source, options)
        .map_err(|err| format!("{}: {err}", String::from_utf8_lossy(path)))?;
    Ok(printer.ctx.buffer.list)
}

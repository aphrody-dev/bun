//! Web APIs: `src/runtime/webcore` on `bun_js`. The Fetch standard: `Headers`, `Request`, `Response`, `fetch()`.

#[path = "Blob.rs"]
pub mod blob;
#[path = "Body.rs"]
pub mod body;
pub mod fetch;
pub mod headers;
#[path = "Request.rs"]
pub mod request;
#[path = "Response.rs"]
pub mod response;

use boa_engine::{Context, JsResult};

pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    headers::install(context)?;
    request::install(context)?;
    response::install(context)?;
    fetch::install(context)
}

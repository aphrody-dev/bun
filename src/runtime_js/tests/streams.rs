//! WHATWG Streams on bun_js. Each expected value is what Bun on JavaScriptCore answers for the same async body.

use bun_js::JSGlobalObject;
use bun_sema_standalone as _;

/// What each async body returns as a string, or `threw <name>: <message>`, once its promises have settled.
fn check(cases: &[(&str, &str)]) {
    JSGlobalObject::with_new(|global| {
        // SAFETY: nothing else reaches the realm while it installs.
        bun_runtime_js::install(unsafe { global.context() }).expect("install");
        for (name, value) in bun_runtime_js::webcore::blob::exports(global).expect("Blob") {
            if name == "Blob" {
                global.to_js_value().put(global, name, value);
            }
        }
        let bun = bun_js::JSObject::create(global);
        for (name, value) in bun_runtime_js::web::streams::readable_stream::bun_functions(global) {
            bun.put(global, name, value);
        }
        global.to_js_value().put(global, "Bun", bun.to_js());
        let mut failures = Vec::new();
        for &(source, expected) in cases {
            let script = format!(
                "globalThis.out = undefined; (async () => {{ {source} }})().then(v => {{ globalThis.out = String(v) }}, e => {{ globalThis.out = 'threw ' + e?.name + ': ' + e?.message }})"
            );
            if let Err(error) = global.evaluate_script(&script) {
                let thrown = global.take_exception(error).to_utf8(global).unwrap_or_default();
                failures.push(format!("{source}\n    uncaught: {thrown}"));
                continue;
            }
            global.drain_microtasks().expect("microtasks");
            let actual = global
                .to_js_value()
                .get(global, "out")
                .expect("out")
                .map(|out| out.to_utf8(global).expect("a string"))
                .unwrap_or_else(|| "did not settle".to_owned());
            if actual != expected {
                failures.push(format!("{source}\n    expected: {expected}\n    actual:   {actual}"));
            }
        }
        assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
    });
}

#[test]
fn readable_writable_and_transform_streams() {
    check(&[
        (
            "const s = new ReadableStream({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close() } }); const r = s.getReader(); const out = []; for (;;) { const { value, done } = await r.read(); if (done) break; out.push(value) } return out.join() + ':' + s.locked",
            "a,b:true",
        ),
        (
            "let n = 0; const s = new ReadableStream({ pull(c) { n++; c.enqueue(n); if (n === 3) c.close() } }, { highWaterMark: 0 }); const out = []; for await (const v of s) out.push(v); return out.join() + ':' + n",
            "1,2,3:3",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2); c.close() } }); const [a, b] = s.tee(); const x = []; for await (const v of a) x.push(v); const y = []; for await (const v of b) y.push(v); return x.join() + '|' + y.join() + '|' + s.locked",
            "1,2|1,2|true",
        ),
        (
            "let reason; const s = new ReadableStream({ cancel(r) { reason = r } }); await s.cancel('why'); return reason",
            "why",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.error(new Error('boom')) } }); try { await s.getReader().read() } catch (e) { return e.message }",
            "boom",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.enqueue('x') } }); const r = s.getReader(); await r.read(); r.releaseLock(); return s.locked + ':' + (await r.closed.catch(e => e.message))",
            "false:Invalid state: Reader released",
        ),
        (
            "let c; const s = new ReadableStream({ start(x) { c = x } }, { highWaterMark: 3 }); const d = [c.desiredSize]; c.enqueue(1); d.push(c.desiredSize); c.close(); return d.join() + ':' + c.desiredSize",
            "3,2:2",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close() } }); return await s.text()",
            "ab",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.enqueue(new Uint8Array([1,2])); c.enqueue(new Uint8Array([3])); c.close() } }); return (await s.bytes()).join()",
            "1,2,3",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.enqueue('{\"a\":'); c.enqueue('1}'); c.close() } }); return (await s.json()).a",
            "1",
        ),
        (
            "const chunks = []; const w = new WritableStream({ write(c) { chunks.push(c) }, close() { chunks.push('closed') } }); const wr = w.getWriter(); await wr.write('a'); await wr.write('b'); await wr.close(); return chunks.join()",
            "a,b,closed",
        ),
        (
            "const w = new WritableStream({}, { highWaterMark: 2 }); const wr = w.getWriter(); const d = [wr.desiredSize]; wr.write('x'); d.push(wr.desiredSize); await wr.ready; return d.join()",
            "2,1",
        ),
        (
            "let reason; const w = new WritableStream({ abort(r) { reason = r } }); await w.abort('no'); return reason + ':' + w.locked",
            "no:false",
        ),
        (
            "const w = new WritableStream({ write() { throw new Error('bad') } }); const wr = w.getWriter(); const e = await wr.write('x').catch(e => e.message); const c = await wr.closed.catch(e => e.message); return e + ':' + c",
            "bad:bad",
        ),
        (
            "const t = new TransformStream({ transform(chunk, c) { c.enqueue(String(chunk).toUpperCase()) } }); const w = t.writable.getWriter(); w.write('a'); w.write('b'); w.close(); return await t.readable.text()",
            "AB",
        ),
        (
            "const t = new TransformStream(); const w = t.writable.getWriter(); w.write('x'); w.close(); const out = []; for await (const v of t.readable) out.push(v); return out.join()",
            "x",
        ),
        (
            "const out = []; const s = new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2); c.close() } }); await s.pipeTo(new WritableStream({ write(v) { out.push(v) } })); return out.join() + ':' + s.locked",
            "1,2:false",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.enqueue('a'); c.enqueue('b'); c.close() } }); return await s.pipeThrough(new TransformStream({ transform(x, c) { c.enqueue(x + x) } })).text()",
            "aabb",
        ),
        (
            "const s = new ReadableStream({ start(c) { c.error('err') } }); const r = await s.pipeTo(new WritableStream()).catch(e => 'rejected ' + e); return r",
            "rejected err",
        ),
        (
            "const it = new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2) } }).values(); const a = await it.next(); const b = await it.return('done'); const c = await it.next(); return JSON.stringify([a, b, c])",
            "[{\"value\":1,\"done\":false},{\"value\":\"done\",\"done\":true},{\"done\":true}]",
        ),
        (
            "const q = new CountQueuingStrategy({ highWaterMark: 4 }); const b = new ByteLengthQueuingStrategy({ highWaterMark: 8 }); return [q.highWaterMark, q.size(), b.highWaterMark, b.size(new Uint8Array(3)), q.size === new CountQueuingStrategy({ highWaterMark: 1 }).size].join()",
            "4,1,8,3,true",
        ),
        (
            "return [Object.prototype.toString.call(new ReadableStream()), Object.prototype.toString.call(new WritableStream()), Object.prototype.toString.call(new TransformStream()), typeof ReadableStream.prototype[Symbol.asyncIterator]].join()",
            "[object ReadableStream],[object WritableStream],[object TransformStream],function",
        ),
        (
            "return await new Blob(['hello ', 'world']).stream().text()",
            "hello world",
        ),
        (
            "const r = new Blob(['ab']).stream().getReader(); const a = await r.read(); const b = await r.read(); return [a.value.constructor.name, a.value.join(), b.done].join()",
            "Uint8Array,97,98,true",
        ),
        (
            "let c; const s = new ReadableStream({ start(x) { c = x } }); const r = s.getReader(); const p = r.read(); c.enqueue('late'); return JSON.stringify(await p)",
            "{\"value\":\"late\",\"done\":false}",
        ),
    ]);
}

#[test]
fn bodies_are_streams() {
    check(&[
        ("const r = new Response('Hello World'); const body = r.body; await r.arrayBuffer(); return [body.locked, await body.blob().catch(e => e.code + ' ' + e.message)].join('|')", "true|ERR_INVALID_STATE Invalid state: ReadableStream has already been used"),
        ("const r = new Response('y'); const b = r.body; const u = [b === r.body, r.bodyUsed]; b.getReader(); u.push(r.bodyUsed); return u.join() + ':' + await r.text().catch(e => e.message)", "true,false,true:Body already used"),
        ("return String(new Response(null).body) + ',' + (new Response('').body === null)", "null,false"),
        ("return await new Response(new ReadableStream({ start(c) { c.enqueue(new Uint8Array([115, 116])); c.close() } })).text()", "st"),
        ("return await new Response('abc').body.text()", "abc"),
        ("const chunks = []; for await (const c of new Response('hi').body) chunks.push(c.constructor.name + c.length); return chunks.join()", "Uint8Array2"),
        ("return await new Request('http://x/', { method: 'POST', body: new ReadableStream({ start(c) { c.enqueue(new Uint8Array([111, 107])); c.close() } }) }).text()", "ok"),
        ("const a = await Bun.readableStreamToText(new Blob(['a', 'b']).stream()); const b = await Bun.readableStreamToArray(new ReadableStream({ start(c) { c.enqueue(1); c.enqueue('x'); c.close() } })); return a + ' ' + b.join()", "ab 1,x"),
    ]);
}

#[test]
fn errors_are_bun_s() {
    check(&[
        ("const s = new ReadableStream(); s.getReader(); try { s.getReader() } catch (e) { return e.code + ' ' + e.message }", "ERR_INVALID_STATE Invalid state: ReadableStream is locked"),
        ("let c; new ReadableStream({ start(x) { c = x } }); c.close(); try { c.enqueue(1) } catch (e) { return e.message }", "Invalid state: Controller is already closed"),
        ("try { new ReadableStream({ type: 'nope' }) } catch (e) { return e.message }", "'nope' is not a valid underlying source 'type'; expected \"bytes\", \"direct\", or undefined"),
        ("try { new ReadableStream({}, { highWaterMark: -1 }) } catch (e) { return e.name + ': ' + e.message }", "RangeError: The queuing strategy's highWaterMark must be a non-negative, non-NaN number"),
        ("try { new ReadableStream({ pull: 1 }) } catch (e) { return e.message }", "The underlying source's 'pull' property must be a function"),
        ("try { new ReadableStream().getReader({ mode: 'x' }) } catch (e) { return e.code + ' ' + e.message }", "ERR_INVALID_ARG_VALUE The property 'options.mode' is invalid. Received 'x'"),
        ("const s = new ReadableStream({ start(c) { c.enqueue(1) } }, { size() { return -1 } }); return 'no'", "threw RangeError: The queuing strategy's chunk size must be a non-negative, finite number"),
        ("const w = new WritableStream(); w.getWriter(); return await w.abort().catch(e => e.message)", "Invalid state: Cannot abort a locked WritableStream"),
        ("const w = new WritableStream(); const wr = w.getWriter(); wr.releaseLock(); return [await wr.write(1).catch(e => e.message), await wr.closed.catch(e => e.message)].join('|')", "Invalid state: Writer is not bound to a WritableStream|Invalid state: Writer has been released"),
        ("try { new CountQueuingStrategy({}) } catch (e) { return e.message }", "QueuingStrategyInit requires a 'highWaterMark' member"),
    ]);
}

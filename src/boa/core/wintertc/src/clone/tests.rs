use crate::test::{TestAction, run_test_actions};
use indoc::indoc;

#[test]
fn errors_keep_their_kind_message_and_stack() {
    run_test_actions([TestAction::run(indoc! {r#"
        const error = new RangeError("out of range");
        const clone = structuredClone({ error, plain: new Error() });
        if (!(clone.error instanceof RangeError) || clone.error.message !== "out of range" ||
            clone.error.stack !== error.stack || clone.error === error) {
            throw new Error("a RangeError did not come back as one");
        }
        if (Object.hasOwn(clone.plain, "message") || !(clone.plain instanceof Error)) {
            throw new Error("an error without a message gained one");
        }
        class Custom extends Error { name = "Custom" }
        if (structuredClone(new Custom("x")).constructor !== Error) {
            throw new Error("an error of another name is not an Error");
        }
    "#})]);
}

#[test]
fn only_enumerable_string_keys_are_cloned() {
    run_test_actions([TestAction::run(indoc! {r#"
        const value = { a: 1, [Symbol("s")]: 2 };
        Object.defineProperty(value, "hidden", { value: 3, enumerable: false });
        const clone = structuredClone(value);
        if (JSON.stringify(Object.getOwnPropertyNames(clone)) !== '["a"]' ||
            Object.getOwnPropertySymbols(clone).length !== 0) {
            throw new Error("a symbol key or a non-enumerable property was cloned");
        }
    "#})]);
}

#[test]
fn failed_serialization_keeps_transfer_buffer_attached() {
    run_test_actions([TestAction::run(indoc! {r#"
        const buffer = new ArrayBuffer(8);
        new Uint8Array(buffer)[0] = 42;

        let threw = false;
        try {
            structuredClone({ buffer, notCloneable() {} }, { transfer: [buffer] });
        } catch {
            threw = true;
        }

        if (!threw || buffer.byteLength !== 8 || new Uint8Array(buffer)[0] !== 42) {
            throw new Error("failed serialization detached or changed the buffer");
        }
    "#})]);
}

#[test]
fn throwing_getter_keeps_transfer_buffer_attached() {
    run_test_actions([TestAction::run(indoc! {r#"
        const buffer = new ArrayBuffer(8);
        const value = {
            buffer,
            get failure() { throw new Error("getter failed"); },
        };

        let threw = false;
        try {
            structuredClone(value, { transfer: [buffer] });
        } catch {
            threw = true;
        }

        if (!threw || buffer.byteLength !== 8) {
            throw new Error("throwing getter detached the buffer");
        }
    "#})]);
}

#[test]
fn successful_transfer_detaches_and_preserves_bytes() {
    run_test_actions([TestAction::run(indoc! {r#"
        const buffer = new ArrayBuffer(8);
        const view = new Uint8Array(buffer);
        view[0] = 42;
        const clone = structuredClone({ buffer, view }, { transfer: [buffer] });

        if (buffer.byteLength !== 0 || clone.buffer.byteLength !== 8 ||
            clone.view.buffer !== clone.buffer || clone.view[0] !== 42) {
            throw new Error("transfer did not preserve the cloned buffer and view");
        }
    "#})]);
}

#[test]
fn unused_transfer_is_detached() {
    run_test_actions([TestAction::run(indoc! {r#"
        const buffer = new ArrayBuffer(8);
        structuredClone({}, { transfer: [buffer] });
        if (buffer.byteLength !== 0) {
            throw new Error("unused transfer buffer was not detached");
        }
    "#})]);
}

#[test]
fn duplicate_transfer_is_rejected_without_detaching() {
    run_test_actions([TestAction::run(indoc! {r#"
        const buffer = new ArrayBuffer(8);
        let threw = false;
        try {
            structuredClone({ buffer }, { transfer: [buffer, buffer] });
        } catch {
            threw = true;
        }

        if (!threw || buffer.byteLength !== 8) {
            throw new Error("duplicate transfer was accepted or detached the buffer");
        }
    "#})]);
}

#[test]
fn detached_transfer_is_checked_in_list_order() {
    run_test_actions([TestAction::run(indoc! {r#"
        function attempt(reverse) {
            const first = new ArrayBuffer(8);
            const second = new ArrayBuffer(8);
            const value = {
                first,
                get detachSecond() {
                    structuredClone(second, { transfer: [second] });
                    return true;
                },
            };

            let threw = false;
            try {
                structuredClone(value, {
                    transfer: reverse ? [second, first] : [first, second],
                });
            } catch {
                threw = true;
            }

            return [threw, first.byteLength, second.byteLength];
        }

        const forward = attempt(false);
        const reverse = attempt(true);
        if (!forward[0] || forward[1] !== 0 || forward[2] !== 0 ||
            !reverse[0] || reverse[1] !== 8 || reverse[2] !== 0) {
            throw new Error("transfer entries were not checked in list order");
        }
    "#})]);
}

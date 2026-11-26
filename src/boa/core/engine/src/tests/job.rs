use std::{
    cell::{Cell, RefCell},
    pin::pin,
    rc::Rc,
};

use futures_lite::future;

use crate::{
    JsValue, TestAction,
    context::{ContextBuilder, time::FixedClock},
    job::{GenericJob, JobExecutor, NativeAsyncJob, SimpleJobExecutor},
    run_test_actions_with,
};

#[test]
fn test_async_job_not_blocking_event_loop() {
    let clock = Rc::new(FixedClock::default());
    let context = &mut ContextBuilder::default()
        .clock(clock.clone())
        .build()
        .unwrap();

    run_test_actions_with(
        [TestAction::inspect_context_async(async move |ctx| {
            let executor = ctx.downcast_job_executor::<SimpleJobExecutor>().unwrap();
            let ctx = &RefCell::new(ctx);

            let mut event_loop = pin!(future::poll_once(executor.run_jobs_async(ctx)));

            // There are no jobs in our queue. Push
            // an async job that will consistently yield to the executor.
            ctx.borrow_mut().enqueue_job(
                NativeAsyncJob::new(async |_| {
                    loop {
                        future::yield_now().await;
                    }
                })
                .into(),
            );

            // Then, start the event loop
            assert!(event_loop.as_mut().await.is_none());

            let checker = Rc::new(Cell::new(false));
            {
                let checker = checker.clone();
                // At this point, the event loop should have yielded again to the async executor.
                // Thus, enqueue a generic job that should resolve in the next loop.
                let realm = ctx.borrow().realm().clone();
                ctx.borrow_mut().enqueue_job(
                    GenericJob::new(
                        move |_| {
                            checker.set(true);
                            Ok(JsValue::undefined())
                        },
                        realm,
                    )
                    .into(),
                );
            }

            // Next iteration of the event loop
            assert!(event_loop.as_mut().await.is_none());

            // At this point, our generic job should have been executed.
            assert!(checker.get());
        })],
        context,
    );
}

/// As HTML's event loop: every promise job, those it queues included, runs before a due timer, and the promise jobs
/// a timer queues run before the next timer.
#[test]
fn promise_jobs_run_before_due_timers() {
    use crate::job::{PromiseJob, TimeoutJob};

    let context = &mut ContextBuilder::default().build().unwrap();
    let order = Rc::new(RefCell::new(Vec::new()));
    let push = |order: &Rc<RefCell<Vec<&'static str>>>, name: &'static str| {
        let order = order.clone();
        move |_: &mut crate::Context| {
            order.borrow_mut().push(name);
            Ok(JsValue::undefined())
        }
    };
    for name in ["first timer", "second timer"] {
        let order = order.clone();
        context.enqueue_job(
            TimeoutJob::from_duration(
                move |context| {
                    order.borrow_mut().push(name);
                    let order = order.clone();
                    context.enqueue_job(
                        PromiseJob::new(move |_| {
                            order.borrow_mut().push("after a timer");
                            Ok(JsValue::undefined())
                        })
                        .into(),
                    );
                    Ok(JsValue::undefined())
                },
                std::time::Duration::ZERO,
            )
            .into(),
        );
    }
    {
        let order = order.clone();
        context.enqueue_job(
            PromiseJob::new(move |context| {
                order.borrow_mut().push("promise job");
                context.enqueue_job(PromiseJob::new(push(&order, "queued by a promise job")).into());
                Ok(JsValue::undefined())
            })
            .into(),
        );
    }
    context.run_jobs().unwrap();
    assert_eq!(
        *order.borrow(),
        [
            "promise job",
            "queued by a promise job",
            "first timer",
            "after a timer",
            "second timer",
            "after a timer"
        ]
    );
}

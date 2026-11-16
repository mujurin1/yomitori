use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

type Work<I, O, E> = dyn Fn(&I) -> Result<O, E> + Send + Sync;
type OnCompleted<I, O> = dyn Fn(I, O) + Send + Sync;
type OnFailed<I, E> = dyn Fn(I, E) + Send + Sync;

/// 要求を 1 件ずつ別スレッドで処理し、処理中に来た要求は最新の 1 件だけ残して古いものを捨てる。
/// 範囲指定中のプレビュー OCR が詰まらないようにするため。結果の通知は処理したスレッドから呼ぶ
pub struct LatestOnlyRunner<I, O, E> {
    shared: Arc<Shared<I, O, E>>,
}

struct Shared<I, O, E> {
    state: Mutex<State<I>>,
    idle: Condvar,
    work: Box<Work<I, O, E>>,
    on_completed: Box<OnCompleted<I, O>>,
    on_failed: Box<OnFailed<I, E>>,
}

struct State<I> {
    running: bool,
    pending: Option<I>,
    stopped: bool,
}

impl<I, O, E> Shared<I, O, E> {
    fn lock(&self) -> MutexGuard<'_, State<I>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<I: Send + 'static, O: 'static, E: 'static> LatestOnlyRunner<I, O, E> {
    pub fn new(
        work: impl Fn(&I) -> Result<O, E> + Send + Sync + 'static,
        on_completed: impl Fn(I, O) + Send + Sync + 'static,
        on_failed: impl Fn(I, E) + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    running: false,
                    pending: None,
                    stopped: false,
                }),
                idle: Condvar::new(),
                work: Box::new(work),
                on_completed: Box::new(on_completed),
                on_failed: Box::new(on_failed),
            }),
        }
    }

    pub fn submit(&self, input: I) {
        let mut state = self.shared.lock();
        if state.stopped {
            return;
        }
        state.pending = Some(input);
        if state.running {
            return;
        }
        state.running = true;
        drop(state);

        let shared = Arc::clone(&self.shared);
        if thread::Builder::new()
            .name("latest-only".into())
            .spawn(move || run_loop(&shared))
            .is_err()
        {
            let mut state = self.shared.lock();
            state.running = false;
            state.pending = None;
            self.shared.idle.notify_all();
        }
    }

    /// 以後の要求を受け付けず、処理中の結果も通知しない
    pub fn stop(&self) {
        let mut state = self.shared.lock();
        state.stopped = true;
        state.pending = None;
    }

    /// 処理のスレッドが終わるまで待つ。timeout までに終われば true。テストや終了待ちに使う
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let state = self.shared.lock();
        let (state, _) = self
            .shared
            .idle
            .wait_timeout_while(state, timeout, |s| s.running)
            .unwrap_or_else(PoisonError::into_inner);
        !state.running
    }
}

/// work や通知が panic してスレッドが落ちても、次の submit で処理を再開できるようにする
struct PanicGuard<'a, I, O, E>(&'a Shared<I, O, E>);

impl<I, O, E> Drop for PanicGuard<'_, I, O, E> {
    fn drop(&mut self) {
        // 正常に抜ける時は running を下ろした後で、もう別のスレッドが走っているかもしれないので触らない
        if thread::panicking() {
            self.0.lock().running = false;
            self.0.idle.notify_all();
        }
    }
}

fn run_loop<I, O, E>(shared: &Shared<I, O, E>) {
    let _guard = PanicGuard(shared);
    loop {
        let input = {
            let mut state = shared.lock();
            match state.pending.take() {
                Some(input) if !state.stopped => input,
                _ => {
                    state.running = false;
                    shared.idle.notify_all();
                    return;
                }
            }
        };

        let result = (shared.work)(&input);
        if shared.lock().stopped {
            continue;
        }
        match result {
            Ok(output) => (shared.on_completed)(input, output),
            Err(error) => (shared.on_failed)(input, error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(5);

    /// 開くまで wait を止めておく門
    #[derive(Clone, Default)]
    struct Gate(Arc<(Mutex<bool>, Condvar)>);

    impl Gate {
        fn open(&self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }

        fn wait(&self) {
            let open = self.0.0.lock().unwrap();
            let _open = self.0.1.wait_while(open, |o| !*o).unwrap();
        }
    }

    fn recorder() -> (Arc<Mutex<Vec<i32>>>, impl Fn(i32) + Send + Sync + 'static) {
        let list = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&list);
        (list, move |n| shared.lock().unwrap().push(n))
    }

    fn values(list: &Arc<Mutex<Vec<i32>>>) -> Vec<i32> {
        list.lock().unwrap().clone()
    }

    #[test]
    fn 処理中に来た要求は最新の1件だけ処理する() {
        let gate = Gate::default();
        let (first_started_tx, first_started) = mpsc::channel();
        let first_started_tx = Mutex::new(first_started_tx);
        let (started, record_started) = recorder();
        let (completed, record_completed) = recorder();
        let work_gate = gate.clone();
        let runner = LatestOnlyRunner::new(
            move |&n: &i32| -> Result<i32, ()> {
                record_started(n);
                if n == 1 {
                    first_started_tx.lock().unwrap().send(()).unwrap();
                    work_gate.wait();
                }
                Ok(n * 10)
            },
            move |n, _| record_completed(n),
            |_, _| {},
        );

        runner.submit(1);
        first_started.recv_timeout(WAIT).unwrap();
        runner.submit(2);
        runner.submit(3);
        runner.submit(4);
        gate.open();
        assert!(runner.wait_idle(WAIT));

        assert_eq!(vec![1, 4], values(&started));
        assert_eq!(vec![1, 4], values(&completed));
    }

    #[test]
    fn 停止後は結果を通知せず新しい要求も受け付けない() {
        let gate = Gate::default();
        let (completed, record_completed) = recorder();
        let work_gate = gate.clone();
        let runner = LatestOnlyRunner::new(
            move |&n: &i32| -> Result<i32, ()> {
                work_gate.wait();
                Ok(n)
            },
            move |n, _| record_completed(n),
            |_, _| {},
        );

        runner.submit(1);
        runner.stop();
        runner.submit(2);
        gate.open();
        assert!(runner.wait_idle(WAIT));

        assert!(values(&completed).is_empty());
    }

    #[test]
    fn 処理の失敗は失敗として通知し次の要求も処理する() {
        let (failed, record_failed) = recorder();
        let (completed, record_completed) = recorder();
        let runner = LatestOnlyRunner::new(
            |&n: &i32| if n == 1 { Err("失敗") } else { Ok(n) },
            move |n, _| record_completed(n),
            move |n, _| record_failed(n),
        );

        runner.submit(1);
        assert!(runner.wait_idle(WAIT));
        runner.submit(2);
        assert!(runner.wait_idle(WAIT));

        assert_eq!(vec![1], values(&failed));
        assert_eq!(vec![2], values(&completed));
    }

    #[test]
    fn 処理がpanicしても次の要求は処理する() {
        let (completed, record_completed) = recorder();
        let runner = LatestOnlyRunner::new(
            |&n: &i32| -> Result<i32, ()> {
                assert_ne!(n, 1, "処理の中の panic");
                Ok(n)
            },
            move |n, _| record_completed(n),
            |_, _| {},
        );

        runner.submit(1);
        assert!(runner.wait_idle(WAIT));
        runner.submit(2);
        assert!(runner.wait_idle(WAIT));

        assert_eq!(vec![2], values(&completed));
    }
}

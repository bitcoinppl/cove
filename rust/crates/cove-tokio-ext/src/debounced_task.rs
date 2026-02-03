use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use arc_swap::ArcSwapOption;
use tracing::debug;

use crate::abortable_task::AbortableTask;

#[derive(Debug)]
pub struct DebouncedTask<T> {
    name: &'static str,
    debounce: Duration,
    task: Arc<ArcSwapOption<AbortableTask<T>>>,
    replace_count: AtomicU64,
    execute_count: Arc<AtomicU64>,
}

impl<T> Clone for DebouncedTask<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            debounce: self.debounce,
            task: self.task.clone(),
            replace_count: AtomicU64::new(self.replace_count.load(Ordering::Relaxed)),
            execute_count: self.execute_count.clone(),
        }
    }
}

impl<T> DebouncedTask<T> {
    pub fn new(name: &'static str, debounce: Duration) -> Self {
        Self {
            name,
            debounce,
            task: Arc::new(ArcSwapOption::from(None)),
            replace_count: AtomicU64::new(0),
            execute_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl<T> DebouncedTask<T>
where
    T: Send + 'static,
{
    pub fn replace<F>(&self, fut: F)
    where
        F: Future<Output = T> + Send + 'static,
    {
        let debounce = self.debounce;
        let name = self.name;
        let replace_num = self.replace_count.fetch_add(1, Ordering::Relaxed) + 1;
        let execute_count = self.execute_count.clone();

        // drop the previous task (aborts it)
        let had_previous = self.task.swap(None).is_some();
        if had_previous {
            debug!("[{name}] debounce cancelled previous pending task (replace #{replace_num})");
        }

        let task = Arc::new(AbortableTask::spawn(async move {
            debug!("[{name}] debounce waiting {debounce:?} (replace #{replace_num})");
            tokio::time::sleep(debounce).await;
            let exec_num = execute_count.fetch_add(1, Ordering::Relaxed) + 1;
            debug!("[{name}] debounce executing (exec #{exec_num}, after replace #{replace_num})");
            fut.await
        }));

        self.task.swap(Some(task));
    }
}

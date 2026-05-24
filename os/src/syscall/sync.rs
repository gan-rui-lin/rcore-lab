use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::current_process;
use alloc::sync::Arc;

pub fn sys_mutex_create(blocking: bool) -> isize {
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    process.with_sync_objects_mut(|inner| {
        if let Some(id) = inner
            .mutex_list
            .iter()
            .enumerate()
            .find(|(_, item)| item.is_none())
            .map(|(id, _)| id)
        {
            inner.mutex_list[id] = mutex;
            id as isize
        } else {
            inner.mutex_list.push(mutex);
            inner.mutex_list.len() as isize - 1
        }
    })
}

pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let process = current_process();
    let mutex =
        process.with_sync_objects(|inner| Arc::clone(inner.mutex_list[mutex_id].as_ref().unwrap()));
    drop(process);
    mutex.lock();
    0
}

pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    let process = current_process();
    let mutex =
        process.with_sync_objects(|inner| Arc::clone(inner.mutex_list[mutex_id].as_ref().unwrap()));
    drop(process);
    mutex.unlock();
    0
}

pub fn sys_semaphore_create(res_count: usize) -> isize {
    let process = current_process();
    let id = process.with_sync_objects_mut(|inner| {
        if let Some(id) = inner
            .semaphore_list
            .iter()
            .enumerate()
            .find(|(_, item)| item.is_none())
            .map(|(id, _)| id)
        {
            inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
            id
        } else {
            inner
                .semaphore_list
                .push(Some(Arc::new(Semaphore::new(res_count))));
            inner.semaphore_list.len() - 1
        }
    });
    id as isize
}

pub fn sys_semaphore_up(sem_id: usize) -> isize {
    let process = current_process();
    let sem = process
        .with_sync_objects(|inner| Arc::clone(inner.semaphore_list[sem_id].as_ref().unwrap()));
    sem.up();
    0
}

pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let process = current_process();
    let sem = process
        .with_sync_objects(|inner| Arc::clone(inner.semaphore_list[sem_id].as_ref().unwrap()));
    sem.down();
    0
}

pub fn sys_condvar_create() -> isize {
    let process = current_process();
    let id = process.with_sync_objects_mut(|inner| {
        if let Some(id) = inner
            .condvar_list
            .iter()
            .enumerate()
            .find(|(_, item)| item.is_none())
            .map(|(id, _)| id)
        {
            inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
            id
        } else {
            inner
                .condvar_list
                .push(Some(Arc::new(Condvar::new())));
            inner.condvar_list.len() - 1
        }
    });
    id as isize
}

pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    let process = current_process();
    let condvar = process
        .with_sync_objects(|inner| Arc::clone(inner.condvar_list[condvar_id].as_ref().unwrap()));
    condvar.signal();
    0
}

pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    let process = current_process();
    let (condvar, mutex) = process.with_sync_objects(|inner| {
        (
            Arc::clone(inner.condvar_list[condvar_id].as_ref().unwrap()),
            Arc::clone(inner.mutex_list[mutex_id].as_ref().unwrap()),
        )
    });
    condvar.wait(mutex);
    0
}

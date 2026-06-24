use core::{num::NonZero, sync::atomic::Ordering};

use alloc::sync::Arc;
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use safa_abi::process::AbiStructures;
use slab::Slab;

use crate::{
    VirtAddr,
    arch::threading::CPUStatus,
    memory::{AlignToPage, paging::MapToError},
    process::{
        DEFAULT_STACK_SIZE, Process,
        mem::{allocate_kernel_stack, allocate_tls, allocate_user_stack},
    },
    thread::{ArcThread, ContextPriority, Thread, Tid},
    utils::elf::TLSInfo,
};

pub struct ThreadsManager {
    thread_ids: Slab<()>,
    threads: HashMap<Tid, ArcThread, FxBuildHasher>,
    userspace_process: bool,
    /// Information about the master TLS if it exits
    master_tls: Option<TLSInfo>,
}

impl ThreadsManager {
    const fn new(userspace_process: bool, master_tls: Option<TLSInfo>) -> Self {
        Self {
            thread_ids: Slab::new(),
            threads: HashMap::with_hasher(FxBuildHasher),
            userspace_process,
            master_tls,
        }
    }

    pub const fn new_uninit() -> Self {
        Self {
            thread_ids: Slab::new(),
            threads: HashMap::with_hasher(FxBuildHasher),
            userspace_process: false,
            master_tls: None,
        }
    }

    /// # Safety:
    /// Once all threads are killed, if this is the current process, no more thread yields can be made.
    pub unsafe fn kill_all(&mut self) {
        let mut current = None;
        for (_, thread) in &self.threads {
            let is_current = crate::thread::is_current(thread);

            // kill current the last, so we are allowed to yield in the meantime
            if is_current {
                current = Some(thread);
                continue;
            }

            if !thread.is_dead() {
                _ = unsafe { thread.soft_kill(true) };
            }
        }

        if let Some(curr) = current {
            unsafe {
                _ = curr.soft_kill(true);
            }
        }
    }

    pub fn new_with_root_thread(
        process: &mut Arc<Process>,
        entry_point: VirtAddr,
        custom_stack_size: Option<NonZero<usize>>,
        args: &[&str],
        env: &[&[u8]],
        abi_structures: AbiStructures,
        userspace_process: bool,
        master_tls: Option<TLSInfo>,
    ) -> Result<(Self, ArcThread), MapToError> {
        let mut this = Self::new(userspace_process, master_tls);
        assert_eq!(this.next_tid(), 0);

        let process_mut = Arc::get_mut(process)
            .expect("More than one reference while trying to create a ThreadsManager");
        let vmm = &mut process_mut.vmm;
        let page_table = process_mut.page_table.get_mut();

        let (
            user_stack_tracker,
            (stack_end, envv_pointers_start, argv_pointers_start, abi_structures_start),
        ) = super::mem::allocate_root_user_env(
            vmm.clone(),
            custom_stack_size
                .map(|s| s.get())
                .unwrap_or(DEFAULT_STACK_SIZE),
            env,
            args,
            abi_structures,
        )?;

        let (ke_stack_tracker, ke_stack_end) = allocate_kernel_stack(DEFAULT_STACK_SIZE)?;
        let tls_allocation = this
            .master_tls
            .map(|tls| {
                allocate_tls(
                    vmm.clone(),
                    tls.addr,
                    tls.alignment,
                    tls.memsize,
                    tls.filesize,
                )
            })
            .transpose()?;

        assert!(stack_end.is_multiple_of(16));
        assert!(ke_stack_end.is_multiple_of(16));

        let entry_args = [
            args.len(),
            argv_pointers_start.into_raw(),
            env.len(),
            envv_pointers_start.into_raw(),
            abi_structures_start.into_raw(),
        ];

        let context = unsafe {
            CPUStatus::create_root(
                page_table,
                entry_point,
                entry_args,
                tls_allocation
                    .as_ref()
                    .map(|(_, addr)| *addr)
                    .unwrap_or(VirtAddr::null()),
                stack_end,
                ke_stack_end,
                this.userspace_process,
            )?
        };

        let next_tid = this.new_tid();
        assert_eq!(next_tid, 0);
        *process_mut.context_count.get_mut() = 1;

        let root_thread = ArcThread::new(Thread::new(
            next_tid,
            context,
            process,
            process.default_priority,
            ke_stack_tracker,
            user_stack_tracker,
            tls_allocation.map(|(a, _)| a),
        ));
        this.threads.insert(next_tid, root_thread.clone());
        Ok((this, root_thread))
    }

    pub fn next_tid(&self) -> Tid {
        self.thread_ids.vacant_key() as Tid
    }

    pub fn new_tid(&mut self) -> Tid {
        self.thread_ids.insert(()) as Tid
    }

    pub fn create_thread(
        &mut self,
        parent: &Arc<Process>,
        entry_point: VirtAddr,
        argument_ptr: VirtAddr,
        priority: Option<ContextPriority>,
        custom_stack_size: Option<NonZero<usize>>,
    ) -> Result<(ArcThread, Tid), MapToError> {
        let stack_size = custom_stack_size
            .map(|stack| stack.get().to_next_page())
            .unwrap_or(DEFAULT_STACK_SIZE);
        let tid = self.next_tid();

        let (th_stack_tracker, stack_end) = allocate_user_stack(parent.vmm.clone(), stack_size)?;
        let tls_allocation = self
            .master_tls
            .map(|tls| {
                allocate_tls(
                    parent.vmm.clone(),
                    tls.addr,
                    tls.alignment,
                    tls.memsize,
                    tls.filesize,
                )
            })
            .transpose()?;
        let (ke_stack_tracker, ke_stack_end) = allocate_kernel_stack(DEFAULT_STACK_SIZE)?;

        let page_table = unsafe { &mut *parent.page_table.get() };
        let cpu_status = unsafe {
            CPUStatus::create_child(
                tls_allocation
                    .as_ref()
                    .map(|(_, a)| *a)
                    .unwrap_or(VirtAddr::null()),
                stack_end,
                ke_stack_end,
                page_table,
                entry_point,
                tid,
                argument_ptr.into_ptr::<()>(),
                self.userspace_process,
            )?
        };

        let thread = Thread::new(
            tid,
            cpu_status,
            parent,
            priority.unwrap_or(parent.default_priority),
            ke_stack_tracker,
            th_stack_tracker,
            tls_allocation.map(|(th, _)| th),
        );
        let thread = ArcThread::new(thread);

        let new_tid = self.new_tid();
        self.threads.insert(new_tid, thread.clone());

        parent.context_count.fetch_add(1, Ordering::SeqCst);

        Ok((thread, new_tid))
    }

    pub fn remove(&mut self, tid: Tid) -> bool {
        if let Some(thread) = self.threads.remove(&tid) {
            // Last thread in process, cleanup process.
            if self.is_empty() {
                unsafe { thread.process().cleanup() }
            }
            true
        } else {
            false
        }
    }

    /// Attempts to remove an unoccupied thread ID, returning `true` if the ID was removed,
    /// false if it never existed, Err(thread) if there is a thread already running with that ID.
    pub fn remove_tid(&mut self, tid: Tid) -> Result<bool, &ArcThread> {
        if let Some(thread) = self.threads.get(&tid) {
            Err(thread)
        } else {
            Ok(self.thread_ids.try_remove(tid as usize).is_some())
        }
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }
}

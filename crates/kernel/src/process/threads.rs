use core::{num::NonZero, sync::atomic::Ordering};

use alloc::sync::Arc;
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use safa_abi::process::AbiStructures;
use slab::Slab;

use crate::{
    VirtAddr,
    arch::threading::CPUStatus,
    memory::paging::MapToError,
    process::{Process, resources::ResourceData},
    thread::{ArcThread, ContextPriority, Thread, Tid},
};

pub struct ThreadsManager {
    thread_ids: Slab<()>,
    threads: HashMap<Tid, ArcThread, FxBuildHasher>,
    userspace_process: bool,
    /// Information about the master TLS if it exits
    master_tls: Option<(VirtAddr, usize, usize, usize)>,
}

impl ThreadsManager {
    const fn new(
        userspace_process: bool,
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
    ) -> Self {
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

    pub fn kill_all(&mut self) {
        for (_, thread) in &self.threads {
            if !thread.is_dead() {
                unsafe { thread.soft_kill(true) };
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
        master_tls: Option<(VirtAddr, usize, usize, usize)>,
    ) -> Result<(Self, ArcThread), MapToError> {
        let mut this = Self::new(userspace_process, master_tls);
        assert_eq!(this.next_tid(), 0);

        let process_mut = Arc::get_mut(process)
            .expect("More than one reference while trying to create a ThreadsManager");
        let vasa = process_mut.vasa.get_mut();
        let resources = process_mut.resources.get_mut();

        let (
            thread_mem_tracker,
            stack_end,
            tp_addr,
            envv_pointers_start,
            argv_pointers_start,
            abi_structers_start,
            ke_stack_tracker,
            ke_stack_end,
        ) = Process::allocate_root_thread_memory_inner(
            vasa,
            custom_stack_size,
            this.master_tls,
            args,
            env,
            abi_structures,
        )?;

        assert!(stack_end.is_multiple_of(16));
        assert!(ke_stack_end.is_multiple_of(16));

        let entry_args = [
            args.len(),
            argv_pointers_start.into_raw(),
            env.len(),
            envv_pointers_start.into_raw(),
            abi_structers_start.into_raw(),
        ];

        let context = unsafe {
            let root_page_table = &mut vasa.page_table;

            CPUStatus::create_root(
                root_page_table,
                entry_point,
                entry_args,
                tp_addr.unwrap_or(VirtAddr::null()),
                stack_end,
                ke_stack_end,
                this.userspace_process,
            )?
        };

        resources.add_global_resource(ResourceData::TrackedMapping(thread_mem_tracker));
        resources.add_global_resource(ResourceData::TrackedMapping(ke_stack_tracker));

        let next_tid = this.new_tid();
        assert_eq!(next_tid, 0);
        *process_mut.context_count.get_mut() = 1;

        let root_thread = ArcThread::new(Thread::new(
            next_tid,
            context,
            process,
            process.default_priority,
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
        let tid = self.next_tid();
        let mut vasa = parent.vasa();

        let (th_mem_tracker, stack_end, tp_addr, ke_stack_tracker, ke_stack_end) =
            Process::allocate_thread_memory_inner(
                &mut vasa,
                custom_stack_size,
                self.master_tls,
                0,
            )?;

        let page_table = &mut vasa.page_table;

        let cpu_status = unsafe {
            CPUStatus::create_child(
                tp_addr.unwrap_or(VirtAddr::null()),
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
        );
        let thread = ArcThread::new(thread);

        let mut resources = parent.resources_mut();
        let th_mem_ri = resources.add_local_resource(ResourceData::TrackedMapping(th_mem_tracker));
        let ke_stack_ri =
            resources.add_local_resource(ResourceData::TrackedMapping(ke_stack_tracker));

        thread.take_resources(&[th_mem_ri, ke_stack_ri]);

        let new_tid = self.new_tid();
        self.threads.insert(new_tid, thread.clone());

        parent.context_count.fetch_add(1, Ordering::SeqCst);

        Ok((thread, new_tid))
    }

    pub fn remove(&mut self, tid: Tid) -> bool {
        self.threads.remove(&tid).is_some()
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

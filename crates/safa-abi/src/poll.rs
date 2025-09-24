#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct PollEvents(u16);

impl PollEvents {
    /// Waiting for no events, ignored.
    pub const NONE: Self = Self(0);
    /// Waiting for data to be available to read.
    pub const DATA_AVAILABLE: Self = Self(1 << 0);
    /// Waiting for the resource to be writable without blocking.
    pub const CAN_WRITE: Self = Self(1 << 1);
    /// The given resource is disconnected, usually is returned and not awaited for.
    /// reads may still be possible if there is data available.
    pub const DISCONNECTED: Self = Self(1 << 2);
    /// Waiting for all events.
    pub const ALL: Self = Self(u16::MAX);

    pub const fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(&self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(&self) -> bool {
        self.0 == 0
    }

    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(&self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn difference(&self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// The layout of a single entry passed to [`crate::syscalls::SyscallTable::SysIOPoll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PollEntry {
    resource: u32,
    events: PollEvents,
    returned_events: PollEvents,
}

impl PollEntry {
    pub const fn new(resource: u32, events: PollEvents) -> Self {
        Self {
            resource,
            events,
            returned_events: PollEvents::NONE,
        }
    }

    /// Returns the resource ID associated with this poll entry.
    pub const fn resource(&self) -> u32 {
        self.resource
    }

    /// Returns the events associated with this poll entry.
    pub const fn events(&self) -> PollEvents {
        self.events
    }

    /// Returns the events that were returned by the poll operation.
    pub const fn returned_events(&self) -> PollEvents {
        self.returned_events
    }

    /// Returns a mutable reference to the events that were returned by the poll operation.
    pub const fn returned_events_mut(&mut self) -> &mut PollEvents {
        &mut self.returned_events
    }
}

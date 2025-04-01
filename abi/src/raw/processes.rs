use super::Optional;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct TaskMetadata {
    pub stdout: Optional<usize>,
    pub stdin: Optional<usize>,
    pub stderr: Optional<usize>,
}

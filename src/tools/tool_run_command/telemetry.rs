use std::sync::atomic::{AtomicUsize, Ordering};

static GLOBAL_SHELL_USAGE: ShellUsageTelemetry = ShellUsageTelemetry::new();

const FILE_OPERATION_COMMANDS: &[&str] = &[
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "rg",
    "grep",
    "sed",
    "awk",
    "patch",
    "apply_patch",
    "perl",
];

/// Counts shell calls that resemble dedicated workspace file-tool operations.
#[derive(Debug, Default)]
pub struct ShellUsageTelemetry {
    total_calls: AtomicUsize,
    file_operation_calls: AtomicUsize,
}

/// Aggregated shell usage counters for host-side telemetry reporting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellUsageSnapshot {
    /// Total number of shell calls recorded by the tool instance.
    total_calls: usize,
    /// Number of shell calls containing a typical read, search, or edit operation.
    file_operation_calls: usize,
}

impl ShellUsageTelemetry {
    const fn new() -> Self {
        Self {
            total_calls: AtomicUsize::new(0),
            file_operation_calls: AtomicUsize::new(0),
        }
    }

    /// Returns the process-wide counters for host-side telemetry reporting.
    pub fn global() -> &'static Self {
        &GLOBAL_SHELL_USAGE
    }

    /// Records one shell call and classifies likely read, search, or edit usage.
    pub fn record_command(&self, command: &str) {
        self.total_calls.fetch_add(1, Ordering::Relaxed);
        if Self::is_file_operation(command) {
            self.file_operation_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Returns counters that a host telemetry exporter can use for before/after ratios.
    pub fn snapshot(&self) -> ShellUsageSnapshot {
        ShellUsageSnapshot {
            total_calls: self.total_calls.load(Ordering::Relaxed),
            file_operation_calls: self.file_operation_calls.load(Ordering::Relaxed),
        }
    }

    /// Computes the share of shell calls classified as read, search, or edit candidates.
    pub fn file_operation_share(snapshot: ShellUsageSnapshot) -> f64 {
        if snapshot.total_calls == 0 {
            return 0.0;
        }
        snapshot.file_operation_calls as f64 / snapshot.total_calls as f64
    }

    fn is_file_operation(command: &str) -> bool {
        command
            .split_whitespace()
            .any(Self::is_file_operation_token)
    }

    fn is_file_operation_token(token: &str) -> bool {
        let token = token.trim_matches(|character: char| {
            matches!(character, '"' | '\'' | '(' | ')' | ';' | '&' | '|')
        });
        let command_name = token.rsplit(['/', '\\']).next().unwrap_or(token);
        FILE_OPERATION_COMMANDS.contains(&command_name)
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellUsageSnapshot, ShellUsageTelemetry};

    #[test]
    fn telemetry_counts_file_operation_candidates() {
        let telemetry = ShellUsageTelemetry::default();
        telemetry.record_command("cargo test");
        telemetry.record_command("rg run_command src");
        telemetry.record_command("apply_patch < change.patch");

        assert_eq!(
            telemetry.snapshot(),
            ShellUsageSnapshot {
                total_calls: 3,
                file_operation_calls: 2,
            }
        );
    }

    #[test]
    fn telemetry_share_is_zero_without_shell_calls() {
        assert_eq!(
            ShellUsageTelemetry::file_operation_share(ShellUsageSnapshot::default()),
            0.0
        );
    }

    #[test]
    fn telemetry_share_can_compare_before_and_after_file_tools() {
        let before = ShellUsageSnapshot {
            total_calls: 4,
            file_operation_calls: 3,
        };
        let after = ShellUsageSnapshot {
            total_calls: 4,
            file_operation_calls: 1,
        };

        assert!(
            ShellUsageTelemetry::file_operation_share(after)
                < ShellUsageTelemetry::file_operation_share(before)
        );
    }
}

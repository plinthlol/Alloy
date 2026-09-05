// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// crate root. main.rs is a thin wrapper that calls the TUI entry point
// re-exported below; everything else stays crate-private. integration tests
// in tests/ that need to reach in deeper can use `alloy::auth`, `alloy::net`,
// etc. directly.

pub mod auth;
pub mod config;
pub mod instance;
pub mod instance_logs;
pub mod launch_profile;
pub mod net;
pub mod running;
pub mod tui;
pub mod util;

use dcc_core::{Computation, MissReason};
use std::collections::BTreeMap;

pub struct MissExplainer;

impl MissExplainer {
    pub fn explain(current: &Computation, previous: Option<&Computation>) -> MissReason {
        let prev = match previous {
            Some(p) => p,
            None => return MissReason::NoEntryFound,
        };

        if current.command != prev.command {
            return MissReason::CommandChanged {
                old: prev.command.clone(),
                new: current.command.clone(),
            };
        }

        if current.args != prev.args {
            return MissReason::ArgumentsChanged {
                old: prev.args.clone(),
                new: current.args.clone(),
            };
        }

        // Compare inputs
        let prev_inputs: BTreeMap<&str, &str> = prev
            .inputs
            .iter()
            .map(|i| (i.path.as_str(), i.digest.as_str()))
            .collect();
        let curr_inputs: BTreeMap<&str, &str> = current
            .inputs
            .iter()
            .map(|i| (i.path.as_str(), i.digest.as_str()))
            .collect();

        for (path, curr_digest) in &curr_inputs {
            match prev_inputs.get(path) {
                Some(prev_digest) if prev_digest != curr_digest => {
                    return MissReason::InputChanged {
                        path: path.to_string(),
                        old_digest: Some(prev_digest.to_string()),
                        new_digest: curr_digest.to_string(),
                    };
                }
                None => {
                    return MissReason::InputAdded {
                        path: path.to_string(),
                    };
                }
                _ => {}
            }
        }

        for path in prev_inputs.keys() {
            if !curr_inputs.contains_key(path) {
                return MissReason::InputRemoved {
                    path: path.to_string(),
                };
            }
        }

        // Compare environment
        for (k, curr_val) in &current.env {
            match prev.env.get(k) {
                Some(prev_val) if prev_val != curr_val => {
                    return MissReason::EnvironmentChanged {
                        key: k.clone(),
                        old: Some(prev_val.clone()),
                        new: Some(curr_val.clone()),
                    };
                }
                None => {
                    return MissReason::EnvironmentChanged {
                        key: k.clone(),
                        old: None,
                        new: Some(curr_val.clone()),
                    };
                }
                _ => {}
            }
        }

        for (k, prev_val) in &prev.env {
            if !current.env.contains_key(k) {
                return MissReason::EnvironmentChanged {
                    key: k.clone(),
                    old: Some(prev_val.clone()),
                    new: None,
                };
            }
        }

        if current.tool != prev.tool {
            let reason = match (&current.tool, &prev.tool) {
                (Some(c), Some(p)) => {
                    let mut diffs = Vec::new();
                    if c.name != p.name {
                        diffs.push(format!("tool name changed: '{}' -> '{}'", p.name, c.name));
                    }
                    if c.version != p.version {
                        diffs.push(format!(
                            "tool version changed: {:?} -> {:?}",
                            p.version, c.version
                        ));
                    }
                    if c.digest != p.digest {
                        diffs.push(format!(
                            "tool digest changed: {:?} -> {:?}",
                            p.digest.as_ref().map(|d| d.as_str()),
                            c.digest.as_ref().map(|d| d.as_str())
                        ));
                    }
                    diffs.join(", ")
                }
                (Some(c), None) => format!("tool identity added: '{}'", c.name),
                (None, Some(p)) => format!("tool identity removed: '{}'", p.name),
                (None, None) => "tool identity changed".to_string(),
            };
            return MissReason::ToolChanged { reason };
        }

        if current.platform != prev.platform {
            let mut diffs = Vec::new();
            if current.platform.os != prev.platform.os {
                diffs.push(format!(
                    "OS changed: '{}' -> '{}'",
                    prev.platform.os, current.platform.os
                ));
            }
            if current.platform.arch != prev.platform.arch {
                diffs.push(format!(
                    "Arch changed: '{}' -> '{}'",
                    prev.platform.arch, current.platform.arch
                ));
            }
            if current.platform.target != prev.platform.target {
                diffs.push(format!(
                    "Target triple changed: {:?} -> {:?}",
                    prev.platform.target, current.platform.target
                ));
            }
            if current.platform.runtime != prev.platform.runtime {
                diffs.push(format!(
                    "Runtime changed: {:?} -> {:?}",
                    prev.platform.runtime, current.platform.runtime
                ));
            }
            if current.platform.abi != prev.platform.abi {
                diffs.push(format!(
                    "ABI changed: {:?} -> {:?}",
                    prev.platform.abi, current.platform.abi
                ));
            }
            if current.platform.compiler != prev.platform.compiler {
                diffs.push(format!(
                    "Compiler changed: {:?} -> {:?}",
                    prev.platform.compiler, current.platform.compiler
                ));
            }
            let reason = if diffs.is_empty() {
                "Platform constraints changed".to_string()
            } else {
                diffs.join(", ")
            };
            return MissReason::PlatformChanged { reason };
        }

        MissReason::NoEntryFound
    }
}

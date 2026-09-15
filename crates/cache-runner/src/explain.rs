use std::collections::{BTreeMap, HashSet};
use dcc_core::{Computation, MissReason};

pub struct MissExplainer;

impl MissExplainer {
    pub fn explain(
        current: &Computation,
        previous: Option<&Computation>,
    ) -> MissReason {
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
            return MissReason::ToolChanged {
                reason: format!("Current: {:?}, Previous: {:?}", current.tool, prev.tool),
            };
        }

        if current.platform != prev.platform {
            return MissReason::PlatformChanged {
                reason: format!("Current: {:?}, Previous: {:?}", current.platform, prev.platform),
            };
        }

        MissReason::NoEntryFound
    }
}

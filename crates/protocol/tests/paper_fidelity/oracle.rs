use protocol::compact::{ClientCommand, Entry};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub(crate) struct LabeledSequence<'a> {
    fixture: &'static str,
    entries: &'a [Entry],
}

impl<'a> LabeledSequence<'a> {
    pub(crate) fn new(fixture: &'static str, entries: &'a [Entry]) -> Self {
        Self { fixture, entries }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OracleError {
    EmptySequence {
        fixture: &'static str,
    },
    MissingGenesis {
        fixture: &'static str,
    },
    DuplicateGenesis {
        fixture: &'static str,
        position: usize,
    },
    NullEntry {
        fixture: &'static str,
        position: usize,
    },
    UnsupportedNop {
        fixture: &'static str,
        position: usize,
        value: u64,
    },
    DuplicateCommand {
        fixture: &'static str,
        first_position: usize,
        duplicate_position: usize,
        command: ClientCommand,
    },
}

pub(crate) fn median_sequence(
    mut sequences: [LabeledSequence<'_>; 3],
) -> Result<Vec<Entry>, OracleError> {
    for sequence in &sequences {
        validate(sequence)?;
    }
    sequences.sort_by(|left, right| compare_commands(left.entries, right.entries));
    Ok(sequences[1].entries.to_vec())
}

fn validate(sequence: &LabeledSequence<'_>) -> Result<(), OracleError> {
    let Some(first) = sequence.entries.first() else {
        return Err(OracleError::EmptySequence {
            fixture: sequence.fixture,
        });
    };
    match first {
        Entry::Nop(0) => {}
        Entry::Nop(value) => {
            return Err(OracleError::UnsupportedNop {
                fixture: sequence.fixture,
                position: 0,
                value: *value,
            });
        }
        Entry::Null { .. } => {
            return Err(OracleError::NullEntry {
                fixture: sequence.fixture,
                position: 0,
            });
        }
        Entry::Cmd(_) => {
            return Err(OracleError::MissingGenesis {
                fixture: sequence.fixture,
            });
        }
    }

    let mut seen = BTreeMap::new();
    for (position, entry) in sequence.entries.iter().enumerate().skip(1) {
        match entry {
            Entry::Cmd(command) => {
                if let Some(first_position) = seen.insert(*command, position) {
                    return Err(OracleError::DuplicateCommand {
                        fixture: sequence.fixture,
                        first_position,
                        duplicate_position: position,
                        command: *command,
                    });
                }
            }
            Entry::Null { .. } => {
                return Err(OracleError::NullEntry {
                    fixture: sequence.fixture,
                    position,
                });
            }
            Entry::Nop(0) => {
                return Err(OracleError::DuplicateGenesis {
                    fixture: sequence.fixture,
                    position,
                });
            }
            Entry::Nop(value) => {
                return Err(OracleError::UnsupportedNop {
                    fixture: sequence.fixture,
                    position,
                    value: *value,
                });
            }
        }
    }
    Ok(())
}

fn compare_commands(left: &[Entry], right: &[Entry]) -> Ordering {
    for (left, right) in left[1..].iter().zip(&right[1..]) {
        let (Entry::Cmd(left), Entry::Cmd(right)) = (left, right) else {
            unreachable!("oracle inputs are validated before comparison")
        };
        let ordering = (left.client, left.sn, left.op).cmp(&(right.client, right.sn, right.op));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

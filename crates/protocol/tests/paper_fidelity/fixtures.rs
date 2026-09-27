use protocol::Config;
use protocol::compact::{ClientCommand, CompactNode, CompactReply, Entry, Timed, Triage};
use protocol::recovery::{RecoveryNode, RecoveryReply};
use std::sync::Arc;

pub(crate) const T_NO_COMMIT: u64 = 100;
pub(crate) const COMMAND_A: ClientCommand = ClientCommand {
    client: 1,
    sn: 1,
    op: 1,
};
pub(crate) const COMMAND_B: ClientCommand = ClientCommand {
    client: 2,
    sn: 1,
    op: 2,
};
pub(crate) const COMMAND_C: ClientCommand = ClientCommand {
    client: 3,
    sn: 1,
    op: 3,
};
pub(crate) const COMMAND_D: ClientCommand = ClientCommand {
    client: 4,
    sn: 1,
    op: 4,
};
pub(crate) const REQUIRED_PREFIX: [Entry; 3] =
    [Entry::Nop(0), Entry::Cmd(COMMAND_A), Entry::Cmd(COMMAND_C)];

pub(crate) struct DonorFixture<R> {
    pub(crate) logs: [Vec<Timed>; 3],
    pub(crate) replies: [R; 6],
    pub(crate) committed: [Vec<Entry>; 3],
}

pub(crate) fn bare(log: &[Timed]) -> Vec<Entry> {
    log.iter().map(|timed| timed.entry.clone()).collect()
}

pub(crate) fn compact_donors() -> DonorFixture<CompactReply> {
    let mut nodes: [CompactNode; 3] =
        std::array::from_fn(|_| CompactNode::new(Config::default(), T_NO_COMMIT));
    for round in 1..=3 {
        for (donor, node) in nodes.iter_mut().enumerate() {
            let commands = scheduled_commands(donor, round);
            for command in commands {
                assert_eq!(
                    node.on_client_command(command),
                    Triage::Amplify,
                    "compact donor {donor} command {command:?} must be a legal append in round {round}"
                );
            }
            let appends: Vec<_> = commands
                .iter()
                .copied()
                .map(|command| (command, round))
                .collect();
            let self_reply = compact_reply(node);
            let replies = [self_reply.clone(), self_reply.clone(), self_reply];
            node.step_chosen(&replies, &appends, round, Some(&[0, 1, 2]));
        }
    }
    compact_fixture(&nodes)
}

pub(crate) fn recovery_donors() -> DonorFixture<RecoveryReply> {
    let mut nodes: [RecoveryNode; 3] =
        std::array::from_fn(|_| RecoveryNode::new(Config::default(), T_NO_COMMIT, false));
    for round in 1..=3 {
        for (donor, node) in nodes.iter_mut().enumerate() {
            let commands = scheduled_commands(donor, round);
            for command in commands {
                assert!(
                    node.wants_amplify(command),
                    "recovery donor {donor} command {command:?} must be a legal append in round {round}"
                );
            }
            let appends: Vec<_> = commands
                .iter()
                .copied()
                .map(|command| (command, round))
                .collect();
            let self_reply = recovery_reply(node);
            let replies = [self_reply.clone(), self_reply.clone(), self_reply];
            node.step_chosen(&replies, &appends, Some(&[0, 1, 2]));
        }
    }
    recovery_fixture(&nodes)
}

fn scheduled_commands(donor: usize, round: u64) -> &'static [ClientCommand] {
    match (donor, round) {
        (0, 2) => &[COMMAND_A, COMMAND_B],
        (1, 1) => &[COMMAND_A],
        (1, 2) => &[COMMAND_C],
        (2, 3) => &[COMMAND_A, COMMAND_D],
        (0..=2, 1..=3) => &[],
        _ => panic!("unknown donor/round schedule: donor {donor}, round {round}"),
    }
}

fn compact_fixture(nodes: &[CompactNode; 3]) -> DonorFixture<CompactReply> {
    let logs = std::array::from_fn(|index| {
        nodes[index]
            .log_entries()
            .expect("a compact donor remains log-bearing")
    });
    let replies = [
        compact_reply(&nodes[0]),
        compact_reply(&nodes[1]),
        compact_reply(&nodes[2]),
        compact_reply(&nodes[0]),
        compact_reply(&nodes[1]),
        compact_reply(&nodes[2]),
    ];
    let committed = std::array::from_fn(|index| nodes[index].shared_state().untruncated().to_vec());
    DonorFixture {
        logs,
        replies,
        committed,
    }
}

fn recovery_fixture(nodes: &[RecoveryNode; 3]) -> DonorFixture<RecoveryReply> {
    let logs = std::array::from_fn(|index| {
        nodes[index]
            .log_entries()
            .expect("a recovery donor remains log-bearing")
    });
    let replies = [
        recovery_reply(&nodes[0]),
        recovery_reply(&nodes[1]),
        recovery_reply(&nodes[2]),
        recovery_reply(&nodes[0]),
        recovery_reply(&nodes[1]),
        recovery_reply(&nodes[2]),
    ];
    let committed = std::array::from_fn(|index| nodes[index].shared_state().untruncated().to_vec());
    DonorFixture {
        logs,
        replies,
        committed,
    }
}

fn compact_reply(node: &CompactNode) -> CompactReply {
    CompactReply::from_log(
        Arc::clone(node.log_arc().expect("donor remains log-bearing")),
        None,
    )
}

fn recovery_reply(node: &RecoveryNode) -> RecoveryReply {
    let log = node.log_perm_arc().expect("donor remains log-bearing");
    RecoveryReply {
        l_j: Some(log.clone()),
        c_j: node.checkpoint_shared(),
        r_j: node.reset_state(),
    }
}

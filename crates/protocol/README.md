# protocol

The rules from Cachin, Dou, Scheideler, Schneider, "A Lightweight Approach for State
Machine Replication" (arXiv:2509.17771v2), as pure per-node state machines. Page numbers
below refer to that version.

A node here never sends or receives anything. The driver (`crates/sim` or
`crates/network-node`) delivers requests and replies, draws which servers to contact,
and calls the node's step function once per round. Box steps that only send messages
are therefore driver steps and have no code in this crate.

Layout:

- `src/alg/` – one file per algorithm box.
- `src/support/` – data structures the rules run on (chunked log, merge, shared
  state). No protocol decision is made there.
- `src/certificates.rs` – §5 certificates (Merkle forest, client chains, Theorem 6
  verification).

## Box to code

Read each file next to its box in the PDF. Every bullet of a box is marked in the source by
a one-line comment that quotes the bullet's opening words, for example
`// Alg 5: "if i receives at least ℓ replies"`, placed where the bullet's code starts. The
anchors of one box appear in the file in the same order as the bullets in the box. The
table lists every anchor; `step_tags` (a unit test in `src/tag_map.rs`) checks that the
anchors in `src/` and this table agree, including their order.

The same names recur in every file: the paper's variables are fields (`x_i`, `l_i`, `s_i`,
`r_i`, `c_i`; a peer's are `l_j`, `s_j`, `c_j`, `r_j`), `None` is ⊥, the reply bullet is
`answer`, and the two reply-count bullets are `at_least_ell_replies` /
`fewer_than_ell_replies` where they are more than a few lines. Bullets that only send
messages run in the driver and are listed as "driver".

| Anchor | File | Function |
|---|---|---|
| driver | | Alg 1 (p. 10): "send k value requests to servers chosen uniformly and independently at random" |
| driver | | Alg 2 (p. 14): "send k value requests to servers chosen uniformly and independently at random" |
| `Alg 1: "if x_i ≠ ⊥ then for any value request received from some server j"` | `alg/median.rs` | `MedianNode::answer` |
| `Alg 2: "if x_i ≠ ⊥ then for any value request received from some server j"` | `alg/median.rs` | `MedianNode::answer` |
| `Alg 1: "if at least ℓ replies are received"` | `alg/median.rs` | `MedianNode::step` |
| `Alg 1: "if less than ℓ replies are received"` | `alg/median.rs` | `MedianNode::step` |
| `Alg 2: "if at least ℓ replies are received"` | `alg/median.rs` | `MedianNode::step_with` |
| `Alg 2: "if less than ℓ replies are received"` | `alg/median.rs` | `MedianNode::step_with` |
| `Alg 2: "set x_i := f(S)", with f = largest (Definition 3.5)` | `alg/priority.rs` | `PriorityNode::step` |
| driver | | Alg 3 (p. 20): "server i sends k log requests to servers chosen uniformly and independently at random" |
| `Alg 3: "for every command x received from a client that is not yet contained in L_i"` | `alg/extended.rs` | `LogNode::wants_amplify` |
| `Alg 3: "if L_i ≠ ⊥ then for any log request received by server i from a server j"` | `alg/extended.rs` | `LogNode::answer` |
| `Alg 3: "if server i receives at least ℓ replies"` | `alg/extended.rs` | `LogNode::step` |
| `Alg 3: "server i chooses a subset M of size ℓ from the received logs"` | `alg/extended.rs` | `LogNode::step` |
| `Alg 3: "server i sets L_i := L'_i ∘ L̄"` | `alg/extended.rs` | `LogNode::step` |
| `Alg 3: "if server i receives less than ℓ logs"` | `alg/extended.rs` | `LogNode::step` |
| driver | | Alg 4 (p. 21): "send k value requests to servers chosen uniformly and independently at random" |
| `Alg 4: "if x_i ≠ ⊥ then for any value request received from some server j"` | `alg/gossip.rs` | `GossipNode::answer` |
| `Alg 4: "if at least one reply was received that contains x"` | `alg/gossip.rs` | `GossipNode::step` |
| `Alg 4: "if less than ℓ values are received"` | `alg/gossip.rs` | `GossipNode::step` |
| driver | | Alg 5 (p. 26): the σ log n append requests of `Triage::Amplify` and the k requests themselves |
| `Alg 5: "for every command x received from a client c"` | `alg/compact.rs` | `CompactNode::on_client_command` |
| `Alg 5: "if L_i ≠ ⊥, sn(x) = sn(c) + 1 and x ∉ L_i"` | `alg/compact.rs` | `CompactNode::on_client_command` |
| `Alg 5: "if L_i ≠ ⊥ and sn(x) = sn(c)"` | `alg/compact.rs` | `CompactNode::on_client_command` |
| `Alg 5: "otherwise, i ignores x"` | `alg/compact.rs` | `CompactNode::on_client_command` |
| `Alg 5: "i sends k requests ... and attaches a bit b_i"` | `alg/compact.rs` | `CompactNode::b_i` |
| `Alg 5: "if L_i ≠ ⊥ then for any request received by i from some server j"` | `alg/compact.rs` | `CompactNode::answer` |
| `Alg 5: "if i receives at least ℓ replies"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "i chooses a subset M of ℓ of the replies uniformly at random"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "i sets L_i := L'_i ∘ L̄"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "if the logs in L_i contain different commands from the same client with the same sequence number"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "if b_i = 1 then i picks any reply S_j"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "i determines the largest prefix P_i of L_i"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "i commits the commands in P_i in the given order on S_i, removes P_i from L_i"` | `alg/compact.rs` | `CompactNode::at_least_ell_replies` |
| `Alg 5: "if i receives less than ℓ replies"` | `alg/compact.rs` | `CompactNode::fewer_than_ell_replies` |
| `Alg 5: "i sets L_i := ⊥"` | `alg/compact.rs` | `CompactNode::fewer_than_ell_replies` |
| `Alg 5: "if at least one reply is received then i picks any one of them"` | `alg/compact.rs` | `CompactNode::fewer_than_ell_replies` |
| driver | | Alg 6 (p. 31): "Send k requests to random servers" |
| `Alg 3: "for every command x received from a client that is not yet contained in L_i"` | `alg/recovery.rs` | `RecoveryNode::wants_amplify` |
| `Alg 6: "If a request was received from server j and R_i ≠ ⊥"` | `alg/recovery.rs` | `RecoveryNode::answer` |
| `Alg 3: "if L_i ≠ ⊥ then for any log request received by server i from a server j"` | `alg/recovery.rs` | `RecoveryNode::answer` |
| `Alg 3: "if server i receives at least ℓ replies"` | `alg/recovery.rs` | `RecoveryNode::extended_median_rule` |
| `Alg 3: "server i chooses a subset M of size ℓ from the received logs"` | `alg/recovery.rs` | `RecoveryNode::extended_median_rule` |
| `Alg 3: "server i sets L_i := L'_i ∘ L̄"` | `alg/recovery.rs` | `RecoveryNode::extended_median_rule` |
| `Alg 3: "if server i receives less than ℓ logs"` | `alg/recovery.rs` | `RecoveryNode::extended_median_rule` |
| `Alg 6: "If at least ℓ replies were received"` | `alg/recovery.rs` | `RecoveryNode::at_least_ell_replies` |
| `Alg 6: "If no-reset was received in one reply"` | `alg/recovery.rs` | `RecoveryNode::at_least_ell_replies` |
| `Alg 6: "let C' = (S', P', W') be the received checkpoint with largest W'"` | `alg/recovery.rs` | `RecoveryNode::at_least_ell_replies` |
| `Alg 6: "If W' > W"` | `alg/recovery.rs` | `RecoveryNode::at_least_ell_replies` |
| `Alg 6: "If less than ℓ replies were received"` | `alg/recovery.rs` | `RecoveryNode::fewer_than_ell_replies` |
| `Alg 6: "If R_i = reset"` | `alg/recovery.rs` | `RecoveryNode::between_windows` |
| `Alg 6: "If L_i ≠ ⊥"` | `alg/recovery.rs` | `RecoveryNode::between_windows` |
| `Alg 6: "for current checkpoint C_i = (S, P, W), commit all commands in P on S_i and remove the prefix P from L_i"` | `alg/recovery.rs` | `RecoveryNode::between_windows` |
| `Alg 6: "make new checkpoint C_i := (S_i, P_i, W')"` | `alg/recovery.rs` | `RecoveryNode::between_windows` |
| `Alg 6: "set R_i := no-reset"` | `alg/recovery.rs` | `RecoveryNode::between_windows` |
| `Alg 6: "If L_i = ⊥"` | `alg/recovery.rs` | `RecoveryNode::between_windows` |

Algorithm 6's "choose C' with the largest W'" is the function `latest_checkpoint`.
`end_window` and `end_window_shared` both run the between-windows bullets through the one
function `between_windows`. `end_window_shared` differs only in "make new checkpoint": instead of building
a new checkpoint it holds an equal one that another node already built (after asserting the
equality). It only saves memory and adds no box step. The operations on P (L_i := P, execute
P, remove P, P_i) are named functions in `support/checkpoint.rs`.

§5 has no algorithm box, so `certificates.rs` has no step tags. Its functions name the
paper objects directly (see the glossary).

## Choices the paper does not fix

Three kinds, each tagged in the source with its id:

- `// DETERMINIZED: [DET-n]` – the box leaves a choice open; the code fixes one.
- `// EXTENSION: [EXT-n]` – the code does more than the box, as the paper's prose allows.
- `// DEVIATION: [DEV-n]` – the code differs from a paper definition on purpose.

The Source column names where the choice is argued. "Design notes" refers to the
author's working notes and experiment logs, which are not part of this repository.

| Id | Kind | Choice | Why | Where | Source |
|---|---|---|---|---|---|
| DET-1 | DETERMINIZED | The median comparator compares entries together with their round stamp. The paper orders command sequences and does not say whether the stamp counts. | Default for both rules. In recovery, a controlled run found stamped-prefix aborts with command-only ordering and none with this one. `StampTie::CommandsOnly` is kept as an option. | `alg/compact.rs`, `alg/recovery.rs` | Design notes (stamp-tie recovery run) |
| DET-2 | DETERMINIZED | A command seen in two non-median logs keeps the stamp of the first log in sorted order. The box lets L̄ be built "in any order". | The first sighting is kept, later copies are dropped. `UnionStamp::LastSighting` is kept as an option. | `alg/compact.rs`, `alg/recovery.rs` | Design notes |
| DET-3 | DETERMINIZED | Alg 5 "if b_i = 1 then i picks any reply S_j": "any S_j in M" is the first reply in draw order that carries a state. | Any choice is allowed; draw order is fixed per seed. | `alg/compact.rs` | Design notes |
| DET-4 | DETERMINIZED | Alg 5 "if at least one reply is received then i picks any one of them": "any one" reply is the first reply received. That one reply gives both the state and the aged prefix. | One reply is the unit, so state and prefix come from the same server. | `alg/compact.rs` | Design notes |
| DET-5 | DETERMINIZED | Under Alg 6 there are two ℓ-counts: Alg 6's "If at least ℓ replies" / "If less than ℓ replies" count every reply, Alg 3's merge counts only replies that carry a log. | Lemma 6.2 (R_i = ⊥ ⇒ L_i = ⊥) is one-way. A node in reset with L_i = ⊥ still answers and must be counted as a reset voter. | `alg/recovery.rs` | Design notes |
| DET-6 | DETERMINIZED | Among received checkpoints with equal largest W', the last reply wins. The box says "break ties arbitrarily". | `max_by_key` keeps the last maximum; `latest_checkpoint` names this. | `alg/recovery.rs` | Design notes |
| DET-7 | DETERMINIZED | In a round, the Alg 3 merge runs first. Checkpoint adoption (Alg 6 "If W' > W") runs after it and overwrites L_i with P'. The box says "on top of extended median rule" and gives no order. | The paper says a server with an older checkpoint "immediately adopts" the newer one (p. 30). | `alg/recovery.rs` | Paper p. 30 |
| DET-8 | DETERMINIZED | The §5 hash is SHA-256 with RFC 6962 leaf/interior prefixes. The paper asks only for a collision-resistant hash (p. 27). | A standard, domain-separated instantiation. | `certificates.rs` | Design notes |
| DET-9 | DETERMINIZED | Committing a command sets sn(c) := sn(x); committing ⊥ for (c, sn) sets sn(c) := sn. The Alg 5 box says the sequence numbers are "incremented". | The two agree whenever a client's commands commit in sequence-number order, which the client bullet enforces (only sn(x) = sn(c) + 1 is amplified). Setting keeps the executed state a function of the committed entries. | `support/shared_state.rs` | Alg 5 box (p. 26) |
| EXT-2 | EXTENSION | §5 certificate state is carried in Alg 6 checkpoints. P is attested when it is executed at the boundary. | §6 says the §5 techniques "can be adapted" (p. 30); P is final only at the boundary (Lemma 6.9). Without certificates (`certs = false`) Alg 6 runs as printed. | `alg/recovery.rs` | Design notes |
| EXT-3 | EXTENSION | Opt-in (`RepeatedCommit::Skip`; the default `Execute` runs the boxes as printed). The replicated state machine is execute-once: a command whose sn(x) ≤ sn(c) leaves S unchanged (cf. Raft client sessions, Ongaro 2014 §6.3). The same holds for ⊥, so sn(c) never moves backwards. This is a property of the state machine, not a change to the protocol: logs, merges and checkpoints are untouched. Used by Alg 5's commit (both reply paths) and Alg 6's boundary commit. Each skipped entry is counted per node, since a skip means agreement already failed upstream. Precondition: a client's commits come in sn order, which holds because a client never has two commands in flight (Alg 5 intake admits only sn(x) = sn(c) + 1). Without it the rule drops a command that was never executed (sn 3 committed before sn 2 makes sn 2 look done). So under `Skip`, a commit with sn(x) > sn(c) + 1 panics. Not combined with §5 certificates (the checkpoint would attest a skipped command again); construction refuses it. | §6 says the techniques of §§4–5, client sequence numbers among them, "can be adapted to work in the recovery protocol" and then assumes unique commands (p. 30); sn(c) is already part of S_i (Alg 5, p. 26). A committed command can re-enter a log after other servers stripped it (a surviving copy re-spreads through the median union) and would otherwise commit a second time. The rule reads only S_i and the entry, so servers with equal states stay equal. It does not reconcile states that already disagree. | `support/shared_state.rs`, `alg/compact.rs`, `alg/recovery.rs` | Design notes |
| DEV-1 | DEVIATION | Alg 6 "If no-reset was received in one reply" sets R_i := no-reset if any reply carries no-reset. Def 3.5 would take the largest of an ℓ-subsample. | The code follows the box and the proof of Lemma 6.4, which scan all replies. Def 3.5 disagrees with its own use here. | `alg/recovery.rs` | Design notes |
| DEV-2 | DEVIATION | Certificate verification also requires the entry to belong to the verifying client. The paper's root case (a) does not check this. | Keeps per-client verification counts from mixing. | `certificates.rs` | Design notes |
| DEV-3 | DEVIATION | Opt-in (`PrefixMismatch::SkipBoundary`; the default `Abort` panics as printed). At a boundary where R_i ≠ reset, L_i ≠ ⊥ and P is not a prefix of L_i, the node skips the between-window steps: nothing is executed, no checkpoint is minted, C_i keeps its old W, and L_i := ⊥ (so R_i := reset by the box's last step). The skipped C_i is marked stale: later boundaries skip the same way (no reset restore, no commit of the stale P) until "If W' > W" adopts a newer peer checkpoint. | Alg 6 (p. 31) has no branch for this case; Lemma 6.7 rules it out only w.h.p., and a near-tie order vote at the minting boundary produces it in practice. Skipping never commits the losing P, and adoption pulls the node onto the majority checkpoint. L_i must go to ⊥: peers strip what they commit at that boundary, and a kept unstripped log re-gossips those commands into every log for a second commit (seen on seed 4331438). | `alg/recovery.rs` | Design notes (prefix-abort diagnosis) |

### Repairs are opt-in; the default is the paper

DEV-3 (`PrefixMismatch::SkipBoundary`) and EXT-3 (`RepeatedCommit::Skip`) stay off unless a
spec turns them on. The default runs Alg 5 and Alg 6 as printed. Reasons:

- Defaults are left out of specs and ledger rows. Changing a default would silently change
  what every stored config means, and commit + seed + config would no longer regenerate a
  stored number.
- The thesis makes claims about the paper's protocol. The repairs are extensions, measured
  against that baseline on the same seeds.

A real deployment would turn both on. DEV-3 alone is not safe: a skipped boundary can let a
command commit twice, and EXT-3 is what stops it.

### Readings the paper's text settles (not tagged)

These follow from the paper's text. They are listed because they are easy to misread.

- L̄ membership is tested by command, not by command plus stamp, and against the growing
  merged log (a set). Otherwise a command could appear twice.
- Alg 5's "performs the updates ... as described above" (fewer than ℓ replies) means: adopt S_j first, then execute the aged prefix. The
  peer's log holds only uncommitted commands.
- b_i is read when the round's step starts, before "i sets L_i := ⊥" can change it.
- Alg 5's client bullet checks `x ∉ L_i` against commands only; a ⊥ entry is not x.
- Alg 5's duplicate bullet ("if the logs in L_i contain different commands from the same
  client with the same sequence number") is part of the box (p. 26) and of §4's prose
  (p. 25). A ⊥ does not block a later command for the same (client, sn); the box is silent
  on that case and no test pins it. (This rule was listed as EXT-1 until 2026-09-27; the id
  is retired.)
- Alg 6 keeps ⊥ and the empty prefix apart: `Checkpoint::p` is `Option<Vec<_>>`. The
  initial P is ⊥.
- The between-windows bullets run in order, not as alternatives: a reset node rolls back and then
  commits P in the same boundary.
- Alg 6 has no duplicate-to-⊥ rule; §6 assumes unique commands (p. 30).
- If P is not a prefix of L_i at the boundary, the node panics by default (DEV-3 is the opt-in skip). The paper assumes this
  never happens w.h.p. (Lemma 6.7); the panic turns a violation into a visible abort.
- Alg 4's last bullet ("if less than ℓ values are received") overrides the one before it;
  the code runs them in box order, so ⊥ wins below ℓ replies.

## Glossary

| Paper | Code |
|---|---|
| n | number of nodes, owned by the driver |
| k, ℓ | `Config::k`, `Config::ell` (ℓ odd, k ≥ ℓ > 1, checked in `Config::new`) |
| x_i, ⊥ (value, Alg 1, 2, 4) | `x_i: Option<Value>` in `MedianNode`, `PriorityNode`, `GossipNode`; `None` is ⊥ |
| f in the (k,ℓ,f)-rule | the `f` argument of `MedianNode::step_with`; `median`, `largest` |
| x, x₀ (gossip) | `GossipNode::x`, `GossipNode::x0` |
| L_i, L_j | `l_i: Option<Log>` in `CompactNode` and `RecoveryNode` (`support/log.rs`: entries plus their sort index); `Option<Arc<Vec<Command>>>` in `LogNode`; a reply's `l_j`. `None` is ⊥ |
| an entry of L_i with its round | `Timed { entry, round }` |
| client command x | `Entry::Cmd(ClientCommand { client, sn, op })` |
| c, sn(x) | `ClientCommand::client`, `ClientCommand::sn` |
| ⊥ as a log entry (§4 duplicate rule) | `Entry::Null { client, sn }` |
| x₀ (seed command), x_d (dummy) | `Entry::Nop(0)` at round 0, `Entry::Nop(round)` (`compact::x_d`) |
| S_i, S_j | `s_i: Arc<SharedState>` (executed commands plus sn(c)); a reply's `s_j` |
| sn(c) | `SharedState::sn_get(client)`, 0 when absent |
| b_i | `CompactNode::b_i()` |
| M | the indices `m`, drawn by `draw_reply_choice` |
| L'_i | the median `logs[at]` from `merge::order_and_pick_median` |
| L'_i ∘ L̄ | `merge::median_merge` (Alg 5; returns a `log::MergedLog`); under Alg 6 `median_union` (`alg/recovery/sharing.rs`); both run `merge::merge_onto_median` |
| T | `t` in `CompactNode` (commit age) and `RecoveryNode` (window length) |
| age ≥ T, P_i | `aged_prefix_len`: the longest prefix with `round - entry.round ≥ T`; Alg 6's P_i is `checkpoint::aged_prefix` |
| R_i ∈ {no-reset, reset, ⊥} | `r_i: RState::{NoReset, Reset, Bot}` (three values, so ⊥ is `Bot`, not `None`) |
| C_i = (S, P, W) | `c_i: Arc<Checkpoint { s, p, w, certs }>`; P = ⊥ is `p: None`; a reply's `c_j` |
| C' = (S', P', W') | `latest_checkpoint(replies)` |
| W' (next window number) | `end_window(next_window, ..)` |
| forest roots h_1..h_a (§5) | `MmrForest::roots()` |
| hc(x), p(x) | `StoredCommand::chain`, `StoredCommand::pos` |
| certificate (x, p(x), h̄c(x)) | `Certificate::Chained`; `Certificate::Newest` when x is the newest command |
| server / client §5 state | `ServerCertState`, `ClientChains` |

Not in the paper: the carried sort index (`perm`), chunked logs (`ChunkSeq`),
copy-on-write state, the draw/apply split (`draw_reply_choice` + `step_chosen`), and the
atomic counters. They exist for speed, memory and deterministic parallel stepping.
`step` equals `draw_reply_choice` followed by `step_chosen`.

## Tests

- `tests/paper/` – one file per rule, tests stated in the paper's terms.
- `tests/paper/reference.rs` – plain reference implementations of Alg 5 and Alg 6 over
  `Vec`s, with the same names and anchors as `src/alg/`; `tests/paper/differential.rs`
  compares them with the production nodes every round. Alg 1–4 need no separate
  reference: their production code already runs on plain values and `Vec`s.
- `tests/recovery_rule.rs`, `tests/paper_fidelity.rs` – Alg 6 rules and fidelity
  controls.
- `tests/support/` – the data structures in `src/support/`.

```bash
cargo test -p protocol
```

//! Golden Shadow-config generation: byte-exact yaml + peers for n=3.

use network_node::deploy::{peers_txt, shadow_yaml};
use network_node::spec::NodeRunSpec;

#[test]
fn median_timer_yaml_n3_is_golden() {
    let spec = NodeRunSpec::from_json(
        r#"{"scenario":{"median":{"n":3,"seed":1,"k":6,"ell":3,
            "init":{"kind":"distinct"},"max_rounds":10}},"sync":"timer"}"#,
    )
    .unwrap();
    let want = "\
general:
  stop_time: 3000 ms
  model_unblocked_syscall_latency: true
network:
  graph:
    type: 1_gbit_switch
hosts:
  peer0:
    network_node_id: 0
    processes:
    - path: node
      args: --spec spec.json --peers peers.txt --node-id 0
      expected_final_state:
        exited: 0
  peer1:
    network_node_id: 0
    processes:
    - path: node
      args: --spec spec.json --peers peers.txt --node-id 1
      expected_final_state:
        exited: 0
  peer2:
    network_node_id: 0
    processes:
    - path: node
      args: --spec spec.json --peers peers.txt --node-id 2
      expected_final_state:
        exited: 0
";
    assert_eq!(shadow_yaml(&spec).unwrap(), want);
    assert_eq!(peers_txt(&spec), "peer0:9000\npeer1:9000\npeer2:9000\n");
}

#[test]
fn compact_barrier_yaml_adds_coordinator_and_client() {
    let spec = NodeRunSpec::from_json(
        r#"{"scenario":{"smr":{"n":3,"seed":1,"k":6,"ell":3,"sigma":1.0,
            "proto":{"kind":"compact","t_commit_rounds":4},"injections":[],
            "max_rounds":10}},"sync":"barrier","round_ms":100}"#,
    )
    .unwrap();
    let yaml = shadow_yaml(&spec).unwrap();
    assert!(
        yaml.contains("coordinator:"),
        "barrier mode ships a coordinator host"
    );
    assert!(
        yaml.contains("--coordinator coordinator:9100"),
        "nodes point at the coordinator host"
    );
    assert!(
        yaml.contains("client:"),
        "SMR mode ships the client driver host"
    );
    assert!(yaml.contains("path: client_driver"));
    assert!(yaml.contains("stop_time: 1500 ms"), "(10+5)·100ms");
}

#[test]
fn nonzero_latency_switches_to_inline_gml() {
    let spec = NodeRunSpec::from_json(
        r#"{"scenario":{"median":{"n":3,"seed":1,"k":6,"ell":3,
            "init":{"kind":"distinct"},"max_rounds":10}},"sync":"timer",
            "network":{"latency_ms":5}}"#,
    )
    .unwrap();
    let yaml = shadow_yaml(&spec).unwrap();
    // Shadow's GML parser wants one attribute per line (its own
    // ONE_GBIT_SWITCH_GRAPH is written that way); a single-line `node [ ... ]`
    // parses as an error at run time, which no `contains` check would catch.
    let want = "network:\n  graph:\n    type: gml\n    inline: |\n\
                \x20     graph [\n\
                \x20       directed 0\n\
                \x20       node [\n\
                \x20         id 0\n\
                \x20         host_bandwidth_up \"1 Gbit\"\n\
                \x20         host_bandwidth_down \"1 Gbit\"\n\
                \x20       ]\n\
                \x20       edge [\n\
                \x20         source 0\n\
                \x20         target 0\n\
                \x20         latency \"5 ms\"\n\
                \x20         packet_loss 0.0\n\
                \x20       ]\n\
                \x20     ]\n";
    assert!(
        yaml.contains(want),
        "gml block must match Shadow's parser format, got:\n{yaml}"
    );
}

#[test]
fn jitter_is_rejected_loudly_until_implemented() {
    let spec = NodeRunSpec::from_json(
        r#"{"scenario":{"median":{"n":3,"seed":1,"k":6,"ell":3,
            "init":{"kind":"distinct"},"max_rounds":10}},"sync":"timer",
            "network":{"jitter_ms":3}}"#,
    )
    .unwrap();
    assert!(
        shadow_yaml(&spec).is_err(),
        "jitter shaping is not implemented — refuse, never silently drop"
    );
}

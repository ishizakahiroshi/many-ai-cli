use many_ai_cli::proto::*;
use serde_json::Value;
#[test]
fn all_go_dto_goldens_roundtrip() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/foundation/proto-golden.json")).unwrap();
    assert_eq!(cases.len(), 44);
    for case in cases {
        let v = case["value"].clone();
        let actual = match case["type"].as_str().unwrap() {
            "Message" => {
                serde_json::to_value(serde_json::from_value::<Message>(v.clone()).unwrap()).unwrap()
            }
            "CrossSessionMessage" => serde_json::to_value(
                serde_json::from_value::<CrossSessionMessage>(v.clone()).unwrap(),
            )
            .unwrap(),
            "AgentChatMessage" => {
                serde_json::to_value(serde_json::from_value::<AgentChatMessage>(v.clone()).unwrap())
                    .unwrap()
            }
            "AgentChatTool" => {
                serde_json::to_value(serde_json::from_value::<AgentChatTool>(v.clone()).unwrap())
                    .unwrap()
            }
            "SessionActivity" => {
                serde_json::to_value(serde_json::from_value::<SessionActivity>(v.clone()).unwrap())
                    .unwrap()
            }
            "WorkflowProgress" => {
                serde_json::to_value(serde_json::from_value::<WorkflowProgress>(v.clone()).unwrap())
                    .unwrap()
            }
            "WfPhase" => {
                serde_json::to_value(serde_json::from_value::<WfPhase>(v.clone()).unwrap()).unwrap()
            }
            "WfAgent" => {
                serde_json::to_value(serde_json::from_value::<WfAgent>(v.clone()).unwrap()).unwrap()
            }
            "WfAgentDetail" => {
                serde_json::to_value(serde_json::from_value::<WfAgentDetail>(v.clone()).unwrap())
                    .unwrap()
            }
            "SubagentTree" => {
                serde_json::to_value(serde_json::from_value::<SubagentTree>(v.clone()).unwrap())
                    .unwrap()
            }
            "SubagentNode" => {
                serde_json::to_value(serde_json::from_value::<SubagentNode>(v.clone()).unwrap())
                    .unwrap()
            }
            "SessionMeta" => {
                serde_json::to_value(serde_json::from_value::<SessionMeta>(v.clone()).unwrap())
                    .unwrap()
            }
            "ApprovalSummary" => {
                serde_json::to_value(serde_json::from_value::<ApprovalSummary>(v.clone()).unwrap())
                    .unwrap()
            }
            "DoneSummary" => {
                serde_json::to_value(serde_json::from_value::<DoneSummary>(v.clone()).unwrap())
                    .unwrap()
            }
            "RelayStatus" => {
                serde_json::to_value(serde_json::from_value::<RelayStatus>(v.clone()).unwrap())
                    .unwrap()
            }
            "RelayEvent" => {
                serde_json::to_value(serde_json::from_value::<RelayEvent>(v.clone()).unwrap())
                    .unwrap()
            }
            "ApprovalOption" => {
                serde_json::to_value(serde_json::from_value::<ApprovalOption>(v.clone()).unwrap())
                    .unwrap()
            }
            "ApprovalRecord" => {
                serde_json::to_value(serde_json::from_value::<ApprovalRecord>(v.clone()).unwrap())
                    .unwrap()
            }
            "ApprovalRecordClose" => serde_json::to_value(
                serde_json::from_value::<ApprovalRecordClose>(v.clone()).unwrap(),
            )
            .unwrap(),
            "ApprovalState" => {
                serde_json::to_value(serde_json::from_value::<ApprovalState>(v.clone()).unwrap())
                    .unwrap()
            }
            "ApprovalSessionState" => serde_json::to_value(
                serde_json::from_value::<ApprovalSessionState>(v.clone()).unwrap(),
            )
            .unwrap(),
            "ChildApproval" => {
                serde_json::to_value(serde_json::from_value::<ChildApproval>(v.clone()).unwrap())
                    .unwrap()
            }
            _ => panic!("unknown fixture type"),
        };
        assert_eq!(actual, v, "{}", case["name"]);
    }
}

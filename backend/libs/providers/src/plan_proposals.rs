//! Terminal draft capability: no owner IDs, existing-plan mutation or approval.
use serde_json::{Value, json};

pub fn tool() -> Value {
    let text = |max: usize| json!({"type":"string","maxLength":max});
    let items = json!({"type":"array","maxItems":16,"items":text(2048)});
    let task = json!({"type":"object","additionalProperties":false,
        "properties":{
            "key":text(64),"objective":text(4096),"scope":text(4096),
            "out_of_scope":text(4096),"acceptance_criteria":items,
            "repository":text(1024),"source_ref":text(256),"execution_limits":text(2048),
            "unresolved_inputs":items
        },
        "required":["key","objective","scope","out_of_scope","acceptance_criteria","repository","source_ref","execution_limits","unresolved_inputs"]});
    json!({"type":"function","name":"propose_plan","strict":true,
        "description":"Propose one new plan in this conversation for human review after discussing scope and constraints. This terminal draft capability cannot approve or execute work. Submit alone, without other tool calls. Keep canonical content within 32768 UTF-8 bytes and all arguments within 40960 bytes. Evidence strings are unverified declarations; identify their source and uncertainty. Put assumptions and unresolved questions explicitly in the draft.",
        "parameters":{"type":"object","additionalProperties":false,
            "properties":{
                "content":{"type":"object","additionalProperties":false,
                    "properties":{"title":text(256),"objective":text(4096),"scope":text(4096),"out_of_scope":text(4096),"constraints":items,"unresolved_questions":items,"tasks":{"type":"array","maxItems":32,"items":task}},
                    "required":["title","objective","scope","out_of_scope","constraints","unresolved_questions","tasks"]},
                "assumptions":items,"evidence":items
            },"required":["content","assumptions","evidence"]}})
}

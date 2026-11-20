//! H3c: what the assistant reads (`docs/ai-assistant.md` §6). The open
//! project's context goes in with each question, and the model may call
//! the read tools MCP has — `list_apis`, `get_api`, `get_variables`,
//! `get_history` — on the workspace's own storage, always about the open
//! project. Same masking and cutting as MCP.
//!
//! H3d: the propose tools — `propose_create_api`, `propose_update_api`,
//! `propose_send` — change and send nothing. Each comes back as a
//! [`Proposal`] the reader applies, sends or discards on a card.

use crate::mcp;
use cyberapi_core::ai::Tool;
use cyberapi_core::mcp::{MAX_TEXT_CHARS, cut, find_folder, listing, written};
use cyberapi_core::model::HttpRequest;
use cyberapi_core::response::HttpResponse;
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_store::Storage;
use serde_json::{Value as Json, json};

/// How many times one question may go back to the model with what its
/// tools answered.
pub const MAX_STEPS: usize = 6;

/// The tools the assistant may call.
pub const READ_TOOLS: [&str; 4] = ["list_apis", "get_api", "get_variables", "get_history"];

/// The tools that only propose, and the MCP ones they are made from.
pub const PROPOSE_TOOLS: [(&str, &str); 3] = [
    ("propose_create_api", "create_api"),
    ("propose_update_api", "update_api"),
    ("propose_send", "send_api"),
];

/// What the model is told a proposal became.
pub const PROPOSED: &str = "Shown to the reader as a card: nothing is changed or sent until they \
    apply or send it, and they may discard it. Do not propose it again; say in a sentence what it does.";

/// What a tool answers at most, in characters.
const MAX_TOOL_CHARS: usize = 4 * MAX_TEXT_CHARS;

/// The tools as MCP describes them, without their `project`: it is
/// always the open one. The propose tools say they only propose.
pub fn tools() -> Vec<Tool> {
    let without_project = |tool: &Json, name: &str, description: String| {
        let mut parameters = tool["inputSchema"].clone();
        if let Some(properties) = parameters["properties"].as_object_mut() {
            properties.remove("project");
            // The reader sends in the environment they chose.
            properties.remove("environment");
        }
        if let Some(required) = parameters["required"].as_array_mut() {
            required.retain(|name| name != "project");
        }
        Tool {
            name: name.to_string(),
            description,
            parameters,
        }
    };
    let all = mcp::tool_list(true, true);
    let named = |name: &str| all.iter().find(|tool| tool["name"] == name);
    let mut tools: Vec<Tool> = READ_TOOLS
        .iter()
        .filter_map(|name| named(name))
        .map(|tool| {
            let name = tool["name"].as_str().unwrap_or_default();
            without_project(tool, name, tool["description"].as_str().unwrap_or_default().to_string())
        })
        .collect();
    for (name, from) in PROPOSE_TOOLS {
        if let Some(tool) = named(from) {
            let description = format!(
                "Proposes this to the reader, who applies it or not: nothing happens until they do. {}",
                tool["description"].as_str().unwrap_or_default()
            );
            tools.push(without_project(tool, name, description));
        }
    }
    tools
}

/// A change or a send the model proposed, for the reader to decide on.
#[derive(Clone, Debug, PartialEq)]
pub enum Proposal {
    /// A new API, `folder` none for the top level.
    Create {
        folder: Option<ItemId>,
        /// Where, as the reader reads it: the folder's path, empty for the top.
        place: String,
        name: String,
        request: Box<HttpRequest>,
    },
    /// An API's request as it is (`before`) and would be; a new name.
    Update {
        api: ItemId,
        name: String,
        rename: Option<String>,
        before: Box<HttpRequest>,
        after: Box<HttpRequest>,
    },
    /// An API sent as it is saved.
    Send { api: ItemId, name: String },
}

/// What the propose tool `name` asks for, read against the open project;
/// or why not.
pub async fn propose(
    storage: &Storage,
    project: &CollectionId,
    name: &str,
    arguments: &Json,
) -> Result<Proposal, String> {
    let mut arguments = match arguments {
        Json::Object(_) => arguments.clone(),
        _ => json!({}),
    };
    arguments["project"] = json!(project.as_str());
    match name {
        "propose_create_api" => {
            let name = arguments["name"]
                .as_str()
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                return Err("the new API needs a name".into());
            }
            let request = written(&HttpRequest::default(), &arguments)?;
            let data = storage
                .tree()
                .load(project.clone())
                .await
                .map_err(|error| error.to_string())?;
            let folder = match arguments["folder"].as_str().filter(|spec| !spec.trim().is_empty()) {
                Some(spec) => Some(find_folder(&data, spec)?),
                None => None,
            };
            let place = folder
                .as_ref()
                .and_then(|folder| listing(&data).into_iter().find(|line| &line.id == folder))
                .map(|line| line.path)
                .unwrap_or_default();
            Ok(Proposal::Create {
                folder,
                place,
                name,
                request: Box::new(request),
            })
        }
        "propose_update_api" => {
            let (_, api, path, ..) = mcp::api(storage, &arguments).await?;
            let after = written(&api.request, &arguments)?;
            let rename = arguments["name"]
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty() && *name != api.name)
                .map(str::to_string);
            if after == api.request && rename.is_none() {
                return Err("that changes nothing: the API already is so".into());
            }
            Ok(Proposal::Update {
                api: api.id,
                name: path,
                rename,
                before: Box::new(api.request),
                after: Box::new(after),
            })
        }
        "propose_send" => {
            let (_, api, path, ..) = mcp::api(storage, &arguments).await?;
            Ok(Proposal::Send {
                api: api.id,
                name: path,
            })
        }
        other => Err(format!("there is no tool {other:?}")),
    }
}

/// Whether `name` is a propose tool.
pub fn proposes(name: &str) -> bool {
    PROPOSE_TOOLS.iter().any(|(tool, _)| *tool == name)
}

/// Calls the tool `name` about `project`; its answer as the model reads
/// it, or why not.
pub async fn call(storage: &Storage, project: &CollectionId, name: &str, arguments: &Json) -> String {
    let mut arguments = match arguments {
        Json::Object(_) => arguments.clone(),
        _ => json!({}),
    };
    arguments["project"] = json!(project.as_str());
    let answer = match name {
        "list_apis" => mcp::list_apis(storage, &arguments).await,
        "get_api" => mcp::get_api(storage, &arguments).await,
        "get_variables" => mcp::get_variables(storage, &arguments).await,
        "get_history" => mcp::get_history(storage, &arguments).await,
        other => Err(format!("there is no tool {other:?}")),
    };
    match answer {
        Ok(json) => cut(&json.to_string(), MAX_TOOL_CHARS),
        Err(why) => format!("error: {why}"),
    }
}

/// The open project as the model is told it with a question: its APIs,
/// the one on screen, and (`response`) the response shown with it.
pub async fn context(
    storage: &Storage,
    project: &CollectionId,
    api: Option<&str>,
    response: Option<&HttpResponse>,
) -> String {
    let mut said = format!(
        "The project's APIs (list_apis):\n{}",
        call(storage, project, "list_apis", &json!({})).await
    );
    if let Some(api) = api {
        said.push_str(&format!(
            "\n\nThe API on the reader's screen (get_api):\n{}",
            call(storage, project, "get_api", &json!({ "api": api })).await
        ));
    }
    if let Some(response) = response {
        said.push_str(&format!(
            "\n\nThe response on the reader's screen:\n{}",
            mcp::response_json(response)
        ));
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::{HttpMethod, HttpRequest};
    use cyberapi_store::{MemoryStore, NewCollection};
    use std::sync::Arc;

    #[test]
    fn the_tools_have_no_project() {
        let tools = tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        let mut expected: Vec<&str> = READ_TOOLS.to_vec();
        expected.extend(PROPOSE_TOOLS.iter().map(|(name, _)| *name));
        assert_eq!(names, expected);
        assert!(
            tools
                .iter()
                .all(|tool| tool.parameters["properties"].get("environment").is_none())
        );
        for tool in &tools {
            assert!(tool.parameters["properties"].get("project").is_none(), "{}", tool.name);
            assert!(
                !tool.parameters["required"].to_string().contains("project"),
                "{}",
                tool.name
            );
        }
    }

    #[test]
    fn the_tools_read_the_open_project() {
        pollster::block_on(reads_the_open_project());
    }

    async fn reads_the_open_project() {
        let storage = Storage::from_backend(Arc::new(MemoryStore::new()));
        let project = storage
            .collections()
            .create(NewCollection {
                id: None,
                name: "shop".into(),
                description: String::new(),
                owner: None,
            })
            .await
            .expect("a project");
        let api = storage
            .tree()
            .create_api(project.id.clone(), None, "login".into())
            .await
            .expect("an API");
        let request = HttpRequest {
            method: HttpMethod::Post,
            uri: "{{HOST}}/login".into(),
            ..Default::default()
        };
        storage
            .tree()
            .save_request(project.id.clone(), api.item.id.clone(), request)
            .await
            .expect("saved");

        let listed = call(&storage, &project.id, "list_apis", &json!({})).await;
        assert!(listed.contains("login") && listed.contains("POST"), "{listed}");
        let got = call(&storage, &project.id, "get_api", &json!({ "api": "login" })).await;
        assert!(got.contains("{{HOST}}/login"), "{got}");
        assert!(
            call(&storage, &project.id, "send_api", &json!({}))
                .await
                .starts_with("error:")
        );
        assert!(
            call(&storage, &project.id, "get_api", &json!({ "api": "nope" }))
                .await
                .starts_with("error:")
        );

        let said = context(&storage, &project.id, Some(api.item.id.as_str()), None).await;
        assert!(said.contains("list_apis") && said.contains("{{HOST}}/login"), "{said}");

        // Proposing changes nothing.
        let update = propose(
            &storage,
            &project.id,
            "propose_update_api",
            &json!({ "api": "login", "method": "PUT" }),
        )
        .await
        .expect("a proposal");
        let Proposal::Update {
            before, after, rename, ..
        } = update
        else {
            panic!("an update")
        };
        assert_eq!(
            (before.method, after.method, rename),
            (HttpMethod::Post, HttpMethod::Put, None)
        );
        assert!(
            call(&storage, &project.id, "get_api", &json!({ "api": "login" }))
                .await
                .contains("\"POST\""),
            "still a POST"
        );
        assert!(
            propose(
                &storage,
                &project.id,
                "propose_update_api",
                &json!({ "api": "login", "method": "POST" })
            )
            .await
            .is_err(),
            "a change to nothing"
        );
        let create = propose(
            &storage,
            &project.id,
            "propose_create_api",
            &json!({ "name": "logout", "method": "POST", "url": "{{HOST}}/logout" }),
        )
        .await
        .expect("a proposal");
        assert!(matches!(create, Proposal::Create { ref name, folder: None, .. } if name == "logout"));
        assert!(
            propose(&storage, &project.id, "propose_create_api", &json!({}))
                .await
                .is_err()
        );
        let send = propose(&storage, &project.id, "propose_send", &json!({ "api": "login" }))
            .await
            .expect("a proposal");
        assert_eq!(
            send,
            Proposal::Send {
                api: api.item.id.clone(),
                name: "login".into()
            }
        );
        let listed = call(&storage, &project.id, "list_apis", &json!({})).await;
        assert!(!listed.contains("logout"), "nothing was made");
    }
}

//! H3: a conversation with the team's model about the open project
//! (`docs/ai-assistant.md`). It lives as long as the project's workspace
//! and is kept nowhere: closing the project forgets it.

use cyberapi_core::ai::{AiEvent, Chat, Message, Part, Role, Stop};
use cyberapi_core::diff::{self, Compared};
use cyberapi_core::response::HttpResponse;
use cyberapi_core::versions::request_text;
use cyberapi_core::{CollectionId, ItemId};
use cyberapi_http::ai::AiModel;
use cyberapi_http::stop_signal;
use cyberapi_service::assistant::{self, Proposal};
use cyberapi_store::Storage;
use futures::StreamExt as _;
use gpui_kit::{AppContext as _, Context, Task, WeakEntity};
use rust_i18n::t;
use serde_json::Value;
use std::sync::Arc;

/// One turn as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    pub role: Role,
    pub text: String,
    /// Said under the answer, outside its Markdown (an answer cut inside a
    /// code block would take it in): it was stopped, cut short, or failed.
    pub note: Option<String>,
    /// The answer did not come whole: the note says why.
    pub failed: bool,
    /// H3c: what the model read to answer, as the reader is told it.
    pub read: Vec<String>,
    /// H3d: what it proposed, for the reader to apply, send or discard.
    pub cards: Vec<Card>,
}

/// A proposal on the reader's screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub proposal: Proposal,
    /// The request's lines as they are and would be (none to send).
    pub compared: Option<Arc<Compared>>,
    pub state: CardState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardState {
    /// Waiting for the reader.
    Open,
    /// Being written.
    Working,
    /// The API changed after it was proposed: applying would lose that.
    Changed,
    /// Written; an API it made, to delete on undoing it.
    Applied {
        made: Option<ItemId>,
    },
    Undone,
    Discarded,
    /// Handed to the API's editor to send.
    Sent,
    Failed(String),
}

impl Card {
    fn of(proposal: Proposal) -> Self {
        let compared = match &proposal {
            Proposal::Create { request, .. } => Some(diff::compare("", &request_text(request))),
            Proposal::Update { before, after, .. } => Some(diff::compare(&request_text(before), &request_text(after))),
            Proposal::Send { .. } => None,
        };
        Self {
            proposal,
            compared: compared.map(Arc::new),
            state: CardState::Open,
        }
    }
}

impl ChatLine {
    fn new(role: Role, text: String) -> Self {
        Self {
            role,
            text,
            note: None,
            failed: false,
            read: Vec::new(),
            cards: Vec::new(),
        }
    }
}

/// The project a conversation is about, as its workspace gives it.
#[derive(Clone)]
pub struct AssistantProject {
    pub name: String,
    pub id: CollectionId,
    /// The workspace's own: a page's server project reads its copy.
    pub storage: Storage,
}

/// What is on the reader's screen as they ask.
#[derive(Clone, Default)]
pub struct OnScreen {
    pub api: Option<ItemId>,
    pub response: Option<Arc<HttpResponse>>,
}

/// A tool the model asked for in one step.
struct Called {
    id: String,
    name: String,
    arguments: Value,
}

pub struct AssistantState {
    model: Arc<dyn AiModel>,
    project: AssistantProject,
    lines: Vec<ChatLine>,
    /// Whether the response on screen goes with a question.
    with_response: bool,
    /// The answer being read, and what stops it: sent or dropped.
    asking: Option<(Task<()>, futures::channel::oneshot::Sender<()>)>,
}

impl AssistantState {
    pub fn new(model: Arc<dyn AiModel>, project: AssistantProject) -> Self {
        Self {
            model,
            project,
            lines: Vec::new(),
            with_response: true,
            asking: None,
        }
    }

    pub fn lines(&self) -> &[ChatLine] {
        &self.lines
    }

    pub fn asking(&self) -> bool {
        self.asking.is_some()
    }

    /// The card at `at` (its line, its place there).
    pub fn card(&self, at: (usize, usize)) -> Option<&Card> {
        self.lines.get(at.0)?.cards.get(at.1)
    }

    pub fn set_card_state(&mut self, at: (usize, usize), state: CardState, cx: &mut Context<Self>) {
        if let Some(card) = self.lines.get_mut(at.0).and_then(|line| line.cards.get_mut(at.1)) {
            card.state = state;
            cx.notify();
        }
    }

    pub fn with_response(&self) -> bool {
        self.with_response
    }

    pub fn set_with_response(&mut self, with: bool, cx: &mut Context<Self>) {
        self.with_response = with;
        cx.notify();
    }

    /// What the model is told before the conversation.
    fn system(&self) -> String {
        format!(
            "You are the assistant built into CyberAPI, an HTTP API client like Postman. The reader has \
             the project \"{}\" open. Help them understand, write and debug its API requests and responses. \
             What follows is the project as it is now; the tools read more of it (an API's request, the \
             variables, past responses). Read before you answer about something you have not seen, and \
             never invent APIs. You cannot change or send anything yourself: to make or change an API, or \
             to send one, call a propose tool and the reader decides on the card it makes; propose only \
             what they asked for. Values that look secret are masked as ***. Be concise; use Markdown, and \
             fenced code blocks for requests, JSON and commands. Answer in the language the reader writes \
             in (the app's language is {}).",
            self.project.name,
            &*rust_i18n::locale()
        )
    }

    /// Asks `question` about the project and what is `on_screen`; the
    /// answer comes in as it is written, the model reading what it needs.
    pub fn ask(&mut self, question: String, on_screen: OnScreen, cx: &mut Context<Self>) {
        let question = question.trim().to_string();
        if question.is_empty() || self.asking.is_some() {
            return;
        }
        self.lines.push(ChatLine::new(Role::User, question));
        // Answers that failed are not the model's to read again.
        let messages = self
            .lines
            .iter()
            .filter(|line| !line.failed && !line.text.is_empty())
            .map(|line| Message::text(line.role, line.text.clone()))
            .collect();
        let chat = Chat {
            system: self.system(),
            messages,
            tools: assistant::tools(),
        };
        self.lines.push(ChatLine::new(Role::Assistant, String::new()));
        let response = on_screen.response.filter(|_| self.with_response);
        let (stopper, stop) = stop_signal();
        let (model, project) = (self.model.clone(), self.project.clone());
        let task = cx.spawn(async move |this, cx| {
            let context = {
                let project = project.clone();
                cx.background_spawn(async move {
                    assistant::context(
                        &project.storage,
                        &project.id,
                        on_screen.api.as_ref().map(ItemId::as_str),
                        response.as_deref(),
                    )
                    .await
                })
                .await
            };
            let chat = Chat {
                system: format!("{}\n\n{context}", chat.system),
                ..chat
            };
            Self::converse(this.clone(), model, project, chat, stop, cx).await;
            let _ = this.update(cx, |this, cx| {
                this.asking = None;
                cx.notify();
            });
        });
        self.asking = Some((task, stopper));
        cx.notify();
    }

    /// Asks the model, runs the tools it asks for and asks again, until it
    /// answers without one or [`assistant::MAX_STEPS`] runs out.
    async fn converse(
        this: WeakEntity<Self>,
        model: Arc<dyn AiModel>,
        project: AssistantProject,
        mut chat: Chat,
        stop: cyberapi_http::Stop,
        cx: &mut gpui_kit::AsyncApp,
    ) {
        for step in 1..=assistant::MAX_STEPS {
            let (told, mut events) = futures::channel::mpsc::unbounded();
            let call = cx.background_spawn(model.chat(
                chat.clone(),
                Arc::new(move |event| {
                    let _ = told.unbounded_send(event);
                }),
                stop.clone(),
            ));
            let (mut said, mut called) = (String::new(), Vec::new());
            // Ends when the call does: it holds what tells.
            while let Some(event) = events.next().await {
                match &event {
                    AiEvent::Text { text } => said.push_str(text),
                    AiEvent::ToolCall { id, name, arguments } => called.push(Called {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    }),
                    _ => {}
                }
                if this.update(cx, |this, cx| this.take(event, cx)).is_err() {
                    return;
                }
            }
            if let Err(error) = call.await {
                let _ = this.update(cx, |this, cx| {
                    this.fail(&error);
                    cx.notify();
                });
                return;
            }
            if called.is_empty() {
                return;
            }
            if step == assistant::MAX_STEPS {
                let _ = this.update(cx, |this, cx| {
                    this.note(t!("assistant.too_many_steps").to_string());
                    cx.notify();
                });
                return;
            }
            let read: Vec<String> = called
                .iter()
                .filter(|call| !assistant::proposes(&call.name))
                .map(|call| read_label(&call.name, &call.arguments))
                .collect();
            if this
                .update(cx, |this, cx| {
                    if let Some(line) = this.lines.last_mut() {
                        line.read.extend(read);
                    }
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            let answers = {
                let project = project.clone();
                let asked: Vec<(String, Value)> = called
                    .iter()
                    .map(|call| (call.name.clone(), call.arguments.clone()))
                    .collect();
                cx.background_spawn(async move {
                    let mut answers = Vec::new();
                    for (name, arguments) in asked {
                        answers.push(if assistant::proposes(&name) {
                            assistant::propose(&project.storage, &project.id, &name, &arguments)
                                .await
                                .map_err(|why| format!("error: {why}"))
                        } else {
                            Err(assistant::call(&project.storage, &project.id, &name, &arguments).await)
                        });
                    }
                    answers
                })
                .await
            };
            // A proposal becomes a card; the model is told it was shown.
            let mut proposals = Vec::new();
            let answers: Vec<String> = answers
                .into_iter()
                .map(|answer| match answer {
                    Ok(proposal) => {
                        proposals.push(proposal);
                        assistant::PROPOSED.to_string()
                    }
                    Err(said) => said,
                })
                .collect();
            if !proposals.is_empty()
                && this
                    .update(cx, |this, cx| {
                        if let Some(line) = this.lines.last_mut() {
                            line.cards.extend(proposals.into_iter().map(Card::of));
                        }
                        cx.notify();
                    })
                    .is_err()
            {
                return;
            }
            let mut parts = Vec::new();
            if !said.is_empty() {
                parts.push(Part::Text { text: said });
            }
            let mut results = Vec::new();
            for (call, content) in called.into_iter().zip(answers) {
                results.push(Part::ToolResult {
                    id: call.id.clone(),
                    content,
                });
                parts.push(Part::ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                });
            }
            chat.messages.push(Message {
                role: Role::Assistant,
                parts,
            });
            chat.messages.push(Message {
                role: Role::User,
                parts: results,
            });
        }
    }

    fn take(&mut self, event: AiEvent, cx: &mut Context<Self>) {
        match event {
            AiEvent::Text { text } => {
                if let Some(line) = self.lines.last_mut() {
                    line.text.push_str(&text);
                }
            }
            AiEvent::Error { message } => self.fail(&message),
            AiEvent::Done { stop: Stop::Length, .. } => self.note(t!("assistant.cut_short").to_string()),
            AiEvent::ToolCall { .. } | AiEvent::Done { .. } => {}
        }
        cx.notify();
    }

    fn fail(&mut self, why: &str) {
        self.note(t!("assistant.failed", why = why).to_string());
        if let Some(line) = self.lines.last_mut() {
            line.failed = true;
        }
    }

    fn note(&mut self, note: String) {
        if let Some(line) = self.lines.last_mut() {
            line.note = Some(note);
        }
    }

    /// Stops the answer being written; what came stays.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.asking.take().is_some() {
            self.note(t!("assistant.stopped").to_string());
            cx.notify();
        }
    }

    /// Forgets the conversation.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.asking = None;
        self.lines.clear();
        cx.notify();
    }
}

#[cfg(test)]
impl AssistantState {
    /// One with a model that never answers, to put cards in by hand.
    pub fn for_tests(project: AssistantProject) -> Self {
        struct Silent;
        impl AiModel for Silent {
            fn model(&self) -> cyberapi_http::ai::AiFuture<Option<String>> {
                Box::pin(async { Ok(None) })
            }
            fn chat(
                &self,
                _: Chat,
                _: cyberapi_http::ai::AiTold,
                _: cyberapi_http::Stop,
            ) -> cyberapi_http::ai::AiFuture<()> {
                Box::pin(async { Ok(()) })
            }
        }
        Self::new(Arc::new(Silent), project)
    }

    /// A card for `proposal` on an answer of its own; where it is.
    pub fn push_card(&mut self, proposal: Proposal) -> (usize, usize) {
        let mut line = ChatLine::new(Role::Assistant, String::new());
        line.cards.push(Card::of(proposal));
        self.lines.push(line);
        (self.lines.len() - 1, 0)
    }
}

/// What a tool call read, as the reader is told it.
fn read_label(name: &str, arguments: &Value) -> String {
    let api = arguments["api"].as_str().unwrap_or_default();
    match name {
        "list_apis" => t!("assistant.read_apis").to_string(),
        "get_api" => t!("assistant.read_api", api = api).to_string(),
        "get_variables" => t!("assistant.read_variables").to_string(),
        "get_history" => t!("assistant.read_history", api = api).to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberapi_core::model::{HttpMethod, HttpRequest};
    use cyberapi_http::ai::{AiFuture, AiTold};
    use cyberapi_store::{MemoryStore, NewCollection};
    use gpui_kit::TestAppContext;
    use serde_json::json;
    use std::sync::Mutex;

    /// Answers each call with the next of `scripts` (the last again once
    /// they run out), keeping what it was asked.
    struct Scripted {
        scripts: Mutex<Vec<Vec<AiEvent>>>,
        asked: Arc<Mutex<Vec<Chat>>>,
    }

    impl Scripted {
        fn new(scripts: Vec<Vec<AiEvent>>, asked: Arc<Mutex<Vec<Chat>>>) -> Arc<Self> {
            Arc::new(Self {
                scripts: Mutex::new(scripts),
                asked,
            })
        }
    }

    impl AiModel for Scripted {
        fn model(&self) -> AiFuture<Option<String>> {
            Box::pin(async { Ok(Some("scripted".into())) })
        }
        fn chat(&self, chat: Chat, told: AiTold, _: cyberapi_http::Stop) -> AiFuture<()> {
            self.asked.lock().expect("lock").push(chat);
            let mut scripts = self.scripts.lock().expect("lock");
            let events = if scripts.len() > 1 {
                scripts.remove(0)
            } else {
                scripts[0].clone()
            };
            Box::pin(async move {
                for event in events {
                    told(event);
                }
                Ok(())
            })
        }
    }

    fn done(stop: Stop) -> AiEvent {
        AiEvent::Done {
            stop,
            input_tokens: None,
            output_tokens: None,
        }
    }

    /// A project "shop" with one API, `POST {{HOST}}/login`.
    fn shop() -> (AssistantProject, ItemId) {
        pollster::block_on(async {
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
                .expect("an API")
                .item;
            let request = HttpRequest {
                method: HttpMethod::Post,
                uri: "{{HOST}}/login".into(),
                ..Default::default()
            };
            storage
                .tree()
                .save_request(project.id.clone(), api.id.clone(), request)
                .await
                .expect("saved");
            (
                AssistantProject {
                    name: project.name,
                    id: project.id,
                    storage,
                },
                api.id,
            )
        })
    }

    #[gpui_kit::test]
    async fn a_question_is_answered_as_it_comes(cx: &mut TestAppContext) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted::new(
            vec![vec![
                AiEvent::Text { text: "Hel".into() },
                AiEvent::Text { text: "lo".into() },
                done(Stop::Done),
            ]],
            asked.clone(),
        );
        let (project, api) = shop();
        let state = cx.new(|_| AssistantState::new(model, project));
        let on_screen = OnScreen {
            api: Some(api),
            response: None,
        };
        state.update(cx, |state, cx| state.ask("  Hi  ".into(), on_screen.clone(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            assert!(!state.asking());
            assert_eq!(
                state.lines(),
                &[
                    ChatLine::new(Role::User, "Hi".into()),
                    ChatLine::new(Role::Assistant, "Hello".into()),
                ]
            );
            state.ask("Again".into(), on_screen, cx);
        });
        cx.run_until_parked();
        let asked = asked.lock().expect("lock");
        assert!(asked[0].system.contains("\"shop\""));
        assert!(
            asked[0].system.contains("{{HOST}}/login"),
            "the API on screen goes with the question"
        );
        assert_eq!(
            asked[0].tools.len(),
            assistant::READ_TOOLS.len() + assistant::PROPOSE_TOOLS.len()
        );
        assert_eq!(asked[1].messages.len(), 3, "the conversation so far, then the question");
        assert_eq!(asked[1].messages[1].parts, vec![Part::Text { text: "Hello".into() }]);
    }

    #[gpui_kit::test]
    async fn the_model_reads_with_a_tool_and_answers(cx: &mut TestAppContext) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted::new(
            vec![
                vec![
                    AiEvent::Text {
                        text: "Let me look. ".into(),
                    },
                    AiEvent::ToolCall {
                        id: "c1".into(),
                        name: "get_api".into(),
                        arguments: json!({ "api": "login" }),
                    },
                    done(Stop::Tools),
                ],
                vec![
                    AiEvent::Text {
                        text: "It posts.".into(),
                    },
                    done(Stop::Done),
                ],
            ],
            asked.clone(),
        );
        let (project, _) = shop();
        let state = cx.new(|_| AssistantState::new(model, project));
        state.update(cx, |state, cx| {
            state.ask("What is login?".into(), OnScreen::default(), cx)
        });
        cx.run_until_parked();
        state.update(cx, |state, _| {
            let last = state.lines().last().expect("an answer");
            assert_eq!(last.text, "Let me look. It posts.");
            assert_eq!(last.read.len(), 1);
            assert!(!state.asking());
        });
        let asked = asked.lock().expect("lock");
        assert_eq!(asked.len(), 2, "asked again with what the tool read");
        let messages = &asked[1].messages;
        assert!(matches!(&messages[1].parts[1], Part::ToolCall { name, .. } if name == "get_api"));
        assert!(
            matches!(&messages[2].parts[0], Part::ToolResult { id, content } if id == "c1" && content.contains("{{HOST}}/login"))
        );
    }

    #[gpui_kit::test]
    async fn a_proposal_becomes_a_card_and_changes_nothing(cx: &mut TestAppContext) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted::new(
            vec![
                vec![
                    AiEvent::ToolCall {
                        id: "p1".into(),
                        name: "propose_update_api".into(),
                        arguments: json!({ "api": "login", "method": "PUT" }),
                    },
                    done(Stop::Tools),
                ],
                vec![
                    AiEvent::Text {
                        text: "Proposed.".into(),
                    },
                    done(Stop::Done),
                ],
            ],
            asked.clone(),
        );
        let (project, api) = shop();
        let storage = project.storage.clone();
        let collection = project.id.clone();
        let state = cx.new(|_| AssistantState::new(model, project));
        state.update(cx, |state, cx| {
            state.ask("Make it a PUT".into(), OnScreen::default(), cx)
        });
        cx.run_until_parked();
        state.update(cx, |state, _| {
            let last = state.lines().last().expect("an answer");
            assert_eq!(last.cards.len(), 1);
            assert_eq!(last.cards[0].state, CardState::Open);
            assert!(last.read.is_empty(), "a proposal is not a read");
            assert!(last.cards[0].compared.as_ref().is_some_and(|compared| !compared.same()));
        });
        let asked = asked.lock().expect("lock");
        assert!(
            matches!(&asked[1].messages[2].parts[0], Part::ToolResult { content, .. } if content == assistant::PROPOSED)
        );
        let stored = pollster::block_on(storage.tree().api(collection, api)).expect("an API");
        assert_eq!(stored.request.method, HttpMethod::Post, "nothing changed");
    }

    #[gpui_kit::test]
    async fn a_model_that_keeps_asking_for_tools_is_stopped(cx: &mut TestAppContext) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted::new(
            vec![vec![
                AiEvent::ToolCall {
                    id: "c".into(),
                    name: "list_apis".into(),
                    arguments: json!({}),
                },
                done(Stop::Tools),
            ]],
            asked.clone(),
        );
        let (project, _) = shop();
        let state = cx.new(|_| AssistantState::new(model, project));
        state.update(cx, |state, cx| state.ask("Loop".into(), OnScreen::default(), cx));
        cx.run_until_parked();
        assert_eq!(asked.lock().expect("lock").len(), assistant::MAX_STEPS);
        state.update(cx, |state, _| {
            assert!(state.lines().last().expect("an answer").note.is_some());
        });
    }

    #[gpui_kit::test]
    async fn a_failed_answer_says_why_and_is_not_asked_again(cx: &mut TestAppContext) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let model = Scripted::new(
            vec![vec![AiEvent::Error {
                message: "the model answered 401".into(),
            }]],
            asked.clone(),
        );
        let (project, _) = shop();
        let state = cx.new(|_| AssistantState::new(model, project));
        state.update(cx, |state, cx| state.ask("Hi".into(), OnScreen::default(), cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            let last = state.lines().last().expect("a line");
            assert!(last.failed && last.note.as_deref().is_some_and(|note| note.contains("401")));
            state.ask("Again".into(), OnScreen::default(), cx);
        });
        cx.run_until_parked();
        let roles: Vec<Role> = asked.lock().expect("lock")[1].messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::User], "the failed answer is left out");
    }
}

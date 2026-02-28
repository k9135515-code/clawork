use anyhow::Context;
use async_trait::async_trait;
use clawork_core::EmbeddingProvider;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String>;
}

#[derive(Clone)]
pub struct MockProvider;

#[async_trait]
impl ChatProvider for MockProvider {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        Ok(format!("[mock completion] {prompt}"))
    }
}

#[derive(Clone)]
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessageOut,
}

#[derive(Deserialize)]
struct OpenAiMessageOut {
    content: String,
}

#[async_trait]
impl ChatProvider for OpenAiProvider {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        let req = OpenAiChatRequest {
            model: &self.model,
            messages: vec![OpenAiMessage {
                role: "user",
                content: prompt,
            }],
        };

        let resp = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await
            .context("openai chat request")?;

        let parsed: OpenAiChatResponse = resp.json().await.context("openai parse chat")?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        Ok(content)
    }
}

#[derive(Clone)]
pub enum ProviderRouter {
    Mock(MockProvider),
    OpenAi(OpenAiProvider),
}

#[async_trait]
impl ChatProvider for ProviderRouter {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        match self {
            ProviderRouter::Mock(p) => p.complete(prompt).await,
            ProviderRouter::OpenAi(p) => p.complete(prompt).await,
        }
    }
}

#[derive(Clone)]
pub struct OpenAiEmbeddingProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiEmbeddingProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let resp = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&EmbeddingRequest {
                model: &self.model,
                input: texts,
            })
            .send()
            .await
            .context("openai embeddings request")?;

        let body: EmbeddingResponse = resp.json().await.context("openai parse embeddings")?;
        Ok(body.data.into_iter().map(|x| x.embedding).collect())
    }
}

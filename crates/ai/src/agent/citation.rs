use std::fmt::Display;

use warp_multi_agent_api as api;

/// A citation listed in an AI response.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum AIAgentCitation {
    WarpDriveObject { uid: String },
    WarpDocumentation { path: String },
    WebPage { url: String },
}

impl Display for AIAgentCitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIAgentCitation::WarpDriveObject { uid } => {
                write!(f, "Zap Drive Object: {uid}")
            }
            AIAgentCitation::WarpDocumentation { path } => {
                write!(f, "Zap Documentation: {path}")
            }
            AIAgentCitation::WebPage { url } => {
                write!(f, "Web Page: {url}")
            }
        }
    }
}

/// Error type for Citation conversion errors
#[derive(Debug, thiserror::Error)]
#[error("Unknown citation type")]
pub struct UnknownCitationTypeError;

impl TryFrom<api::Citation> for AIAgentCitation {
    type Error = UnknownCitationTypeError;

    fn try_from(citation: api::Citation) -> Result<Self, Self::Error> {
        let doc_type = api::DocumentType::try_from(citation.document_type)
            .unwrap_or(api::DocumentType::Unknown);

        match doc_type {
            api::DocumentType::WarpDriveWorkflow
            | api::DocumentType::WarpDriveNotebook
            | api::DocumentType::WarpDriveEnvVar
            | api::DocumentType::Rule => Ok(AIAgentCitation::WarpDriveObject {
                uid: citation.document_id,
            }),
            api::DocumentType::WarpDocumentation => Ok(AIAgentCitation::WarpDocumentation {
                path: citation.document_id,
            }),
            api::DocumentType::WebPage => Ok(AIAgentCitation::WebPage {
                url: citation.document_id,
            }),
            api::DocumentType::Unknown => {
                // 服务端把未知的 citation document_type 映射为 Unknown。记录原始值供诊断
                // (只记 document_id 长度,不含值本身——LLM 派生内容可能携带 URL/路径等
                // 用户数据),整条消息转换不受影响。
                log::warn!(
                    "Citation has an unrecognized document type; dropping it (document_type={}, document_id_len={})",
                    citation.document_type,
                    citation.document_id.len()
                );
                Err(UnknownCitationTypeError)
            }
        }
    }
}

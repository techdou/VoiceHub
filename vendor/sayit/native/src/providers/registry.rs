// 供应商注册表 — Tauri commands 入口

use super::types::*;
use super::{
    ai_ollama, ai_openai_compat, asr_doubao, asr_doubao_stream, asr_groq, asr_mimo, asr_qwen,
    asr_qwen_audio_stream, asr_qwen_omni,
};
use crate::error_protocol;

/// 云端 AI 校对（Tauri command）
#[tauri::command]
pub async fn cloud_polish(request: CloudPolishRequest) -> Result<AiResult, String> {
    let config = &request.ai_config;
    match config.provider.as_str() {
        // Groq 是标准的 OpenAI 兼容 chat/completions，直接复用通用实现，
        // 不需要单独的文件（base_url 已带 /v1，normalize_base_url 会原样保留）。
        "openai_compat" | "deepseek" | "doubao" | "qwen" | "mimo" | "groq" => {
            ai_openai_compat::polish(
                &request.text,
                config,
                request.system_prompt.as_deref(),
                request.text_context.as_ref(),
            )
            .await
        }
        "ollama" => {
            ai_ollama::polish(
                &request.text,
                config,
                request.system_prompt.as_deref(),
                request.text_context.as_ref(),
            )
            .await
        }
        other => Err(error_protocol::encode(
            "connect_failed",
            format!("Unknown AI provider: {}", other),
        )),
    }
}

/// 测试 AI 连接（Tauri command）
#[tauri::command]
pub async fn test_ai_connection(config: AiProviderConfig) -> Result<TestResult, String> {
    match config.provider.as_str() {
        "openai_compat" | "deepseek" | "doubao" | "qwen" | "mimo" | "groq" => {
            Ok(ai_openai_compat::test_connection(&config).await)
        }
        "ollama" => Ok(ai_ollama::test_connection(&config).await),
        other => Err(error_protocol::encode(
            "connect_failed",
            format!("Unknown AI provider: {}", other),
        )),
    }
}

/// 云端 ASR 转写（Tauri command）
#[tauri::command]
pub async fn cloud_transcribe(request: CloudTranscribeRequest) -> Result<AsrResult, String> {
    let config = &request.asr_config;
    match config.provider.as_str() {
        "openai_compat" => super::asr_openai_compat::transcribe(&request.audio_b64, request.sample_rate, config, &request.hotwords).await,
        "doubao" => {
            asr_doubao::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        "doubao_v2" => {
            asr_doubao_stream::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        "qwen" | "aliyun" | "qwen_realtime" => {
            asr_qwen::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        // Qwen-Audio-3.0 流式：关掉实时字幕、以及设置页的识别测试都走这条一次性路径。
        // 用的仍是同一个 duplex 协议，只是整段音频推完再收结果。
        "qwen_audio_stream" => {
            asr_qwen_audio_stream::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        "qwen_omni" => {
            asr_qwen_omni::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        "mimo" => {
            asr_mimo::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        "groq_whisper" => {
            asr_groq::transcribe(
                &request.audio_b64,
                request.sample_rate,
                config,
                &request.hotwords,
            )
            .await
        }
        other => Err(error_protocol::encode(
            "connect_failed",
            format!("ASR provider \"{}\" is not implemented", other),
        )),
    }
}

/// 测试 ASR 连接（Tauri command）
#[tauri::command]
pub async fn test_asr_connection(config: AsrProviderConfig) -> Result<TestResult, String> {
    match config.provider.as_str() {
        "openai_compat" => Ok(super::asr_openai_compat::test_connection(&config).await),
        "doubao" => Ok(asr_doubao::test_connection(&config).await),
        "doubao_v2" => Ok(asr_doubao_stream::test_connection(&config).await),
        "qwen" | "aliyun" | "qwen_realtime" => Ok(asr_qwen::test_connection(&config).await),
        "qwen_audio_stream" => Ok(asr_qwen_audio_stream::test_connection(&config).await),
        "qwen_omni" => Ok(asr_qwen_omni::test_connection(&config).await),
        "mimo" => Ok(asr_mimo::test_connection(&config).await),
        "groq_whisper" => Ok(asr_groq::test_connection(&config).await),
        other => Err(error_protocol::encode(
            "connect_failed",
            format!("ASR provider \"{}\" is not implemented", other),
        )),
    }
}

// 所有 provider 共用的 HTTP 客户端。
//
// 为什么一定要带 User-Agent：**reqwest 默认一个 UA 都不发**，而 nginx / WAF 类网关
// 普遍把「无 UA」当爬虫拦掉。实测 https://test-ai-api.nwcdcloud.cn/v1（LiteLLM 网关）：
// 同一个地址 + 密钥 + 模型，带 UA 返回 200，去掉 UA 头一律 403 并回一段 nginx 的 HTML
// 错误页 —— 请求在网关就被拒了，鉴权根本没执行。我们自己的 ALB 上有同一条规则
// （见 pitfalls 第 1 条）。这类 403 的响应里一个字都不提 UA，所以只会被归成
// 「地区不可用 / 模型没开通」，用户换密钥、换模型都修不掉。
//
// 因此：provider 里**不要再写 `reqwest::Client::new()`**，一律用这里的单例。
// 附带好处是连接池和 TLS 会话得以复用，不必每次调用重建一个客户端。
pub const USER_AGENT: &str = concat!("SayIt/", env!("CARGO_PKG_VERSION"));

static SHARED: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        // 构建失败只可能是 TLS 后端初始化不了，此时退回默认客户端也不会更糟，
        // 至少不要让整个转写功能因为拿不到客户端而不可用。
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// 带 User-Agent 的共享客户端。超时仍按各调用点的 `.timeout()` 单独设置。
pub fn shared() -> &'static reqwest::Client {
    &SHARED
}

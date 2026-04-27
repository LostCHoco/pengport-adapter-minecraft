//! `AppConfig` → PSP `ServiceManifest` 빌더.
//!
//! 어댑터 부팅 시 1회 생성 후 `/.well-known/pengport-service` 응답으로 사용.

use pengport_shared::psp::manifest::{
    CategoryHint, EventType, ManifestEndpoints, NativeActionKind, Permissions, ServiceAction,
    ServiceManifest,
};
use serde_json::json;

use crate::config::AppConfig;

pub fn build_manifest(cfg: &AppConfig, base_url: &str) -> ServiceManifest {
    let mut prism_config = serde_json::Map::new();
    prism_config.insert("host".into(), json!(cfg.mc_host));
    prism_config.insert("port".into(), json!(cfg.mc_port));
    prism_config.insert("version".into(), json!(cfg.mc_version));
    prism_config.insert("loader".into(), json!(cfg.mc_loader));
    if let Some(v) = &cfg.mc_loader_version {
        prism_config.insert("loader_version".into(), json!(v));
    }
    if let Some(v) = &cfg.packwiz_url {
        prism_config.insert("packwiz_url".into(), json!(v));
    }
    if let Some(v) = cfg.mc_java_major {
        prism_config.insert("java_major".into(), json!(v));
    }
    let display = cfg.mc_display_name.clone().unwrap_or_else(|| cfg.mc_name.clone());
    prism_config.insert("display_name".into(), json!(display));

    let mut play_args = serde_json::Map::new();
    play_args.insert("app".into(), json!("prism-launcher"));
    play_args.insert("config".into(), serde_json::Value::Object(prism_config));
    play_args.insert(
        "install_hint".into(),
        json!({
            "name": "Prism Launcher",
            "homepage": "https://prismlauncher.org/",
        }),
    );

    let actions = vec![ServiceAction {
        id: "play".to_string(),
        label: "시작".to_string(),
        primary: true,
        kind: NativeActionKind::ThirdPartyApp,
        args: serde_json::Value::Object(play_args),
    }];

    // packwiz_url 이 있으면 external_urls 에 그 origin pattern 추가.
    // 단순 문자열 처리: scheme://host[:port] 까지 자르고 `/*` 붙임.
    let mut external_urls = Vec::new();
    if let Some(url) = &cfg.packwiz_url {
        if let Some(origin) = origin_of(url) {
            external_urls.push(format!("{}/*", origin));
        }
    }

    let permissions = Permissions {
        native_actions: vec![NativeActionKind::ThirdPartyApp],
        external_urls,
        events: vec![EventType::StatusChanged, EventType::Notification],
    };

    ServiceManifest {
        schema_version: 1,
        psp_version: 1,
        id: cfg.mc_id.clone(),
        name: cfg.mc_name.clone(),
        description: cfg.mc_description.clone(),
        icon_url: cfg.mc_icon_url.clone(),
        category_hint: Some(CategoryHint::Game),
        endpoints: ManifestEndpoints {
            status: format!("{}/pengport/status", base_url.trim_end_matches('/')),
            events: Some(format!("{}/pengport/events", base_url.trim_end_matches('/'))),
        },
        actions,
        permissions,
    }
}

/// "https://host[:port]/path..." → "https://host[:port]". 잘못된 URL 이면 None.
fn origin_of(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    let after_scheme = scheme_end + 3;
    let rest = &url[after_scheme..];
    let host_end = rest.find('/').unwrap_or(rest.len());
    if host_end == 0 {
        return None;
    }
    Some(format!("{}://{}", &url[..scheme_end], &rest[..host_end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_of_https_url() {
        assert_eq!(
            origin_of("https://cdn.example.com/modded/pack.toml"),
            Some("https://cdn.example.com".to_string())
        );
    }

    #[test]
    fn origin_of_with_port() {
        assert_eq!(
            origin_of("https://cdn.example.com:8443/modded/pack.toml"),
            Some("https://cdn.example.com:8443".to_string())
        );
    }

    #[test]
    fn origin_of_no_scheme() {
        assert_eq!(origin_of("cdn.example.com/path"), None);
    }
}

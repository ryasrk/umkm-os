//! P9-5 (ADR-0018): `<head>` situs toko per Host, robots.txt, sitemap.xml. Role aplikasi + RLS.

mod common;

use axum::http::Method;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use umkm_api::{build_router, site_head::SiteHead};
use umkm_database::MIGRATOR;
use uuid::Uuid;

const SHELL: &str = "<!DOCTYPE html><html lang=\"id\"><head><meta charset=\"utf-8\"><title>UMKM OS</title><meta name=\"description\" content=\"shell\"/><script src=\"/_next/static/a.js\"></script></head><body><div id=\"root\"></div></body></html>";

fn studio_doc() -> Value {
    let raw = include_str!("../../../crates/domain/tests/fixtures/studio_browser_doc.json");
    let mut d: Value = serde_json::from_str(raw).unwrap();
    d["template"] = json!("modern_store");
    d["seo"] = json!({"title": "Kopi Senja </title><script>x</script>", "description": "Kopi & roti \"enak\""});
    d
}

async fn get(a: &axum::Router, uri: &str, host: &str) -> Res {
    send(a, Method::GET, uri, None, Some(host), None, vec![]).await
}

/// Seperti Caddy (snippet `web_shell` dari csp.mjs): kirim identitas shell build web yang disajikan.
async fn doc(a: &axum::Router, uri: &str, host: &str, shell: &str) -> Res {
    doc_if(a, uri, host, shell, None).await
}

async fn doc_if(a: &axum::Router, uri: &str, host: &str, shell: &str, inm: Option<&str>) -> Res {
    let mut req = axum::http::Request::builder()
        .uri(uri)
        .header("host", host)
        .header("x-web-shell", umkm_api::site_head::shell_hash(shell));
    if let Some(v) = inm {
        req = req.header("if-none-match", v);
    }
    let req = req.body(axum::body::Body::empty()).unwrap();
    let res = tower::ServiceExt::oneshot(a.clone(), req).await.unwrap();
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Res {
        status,
        headers,
        bytes,
    }
}

async fn publish_doc(a: &TestApp, t: Uuid, o: &Who, doc: Value) {
    let w = format!("/api/v1/tenants/{t}/website");
    let r = json(
        &a.router,
        Method::PUT,
        &w,
        Some(o),
        Some(json!({"template_key": "modern_store"})),
    )
    .await;
    assert!(r.status < 300, "{}", r.json());
    let d = json(
        &a.router,
        Method::GET,
        &format!("{w}/pages/home/draft"),
        Some(o),
        None,
    )
    .await;
    let rev = d.json()["data"]["revision"].as_i64().unwrap();
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{w}/pages/home/draft"),
        Some(o),
        Some(json!({"content": doc, "base_revision": rev})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());
    let r = json(
        &a.router,
        Method::POST,
        &format!("{w}/pages/home/publish"),
        Some(o),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());
    let r = json(
        &a.router,
        Method::PUT,
        &format!("/api/v1/tenants/{t}/business/profile"),
        Some(o),
        Some(json!({"name": "Kopi <Senja>", "description": "Kedai kopi", "whatsapp": "081234567890", "timezone": "Asia/Jakarta"})),
    )
    .await;
    assert!(r.status < 300, "{}", r.json());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn document_injects_escaped_head_per_host_and_never_leaks(pool: PgPool) {
    let a = app(&pool).await;
    let router = build_router(
        a.state.clone().with_site(SiteHead::new(
            Some(SHELL.into()),
            "https://api.ryasai.my.id".into(),
            "https".into(),
        )),
        LIMITS,
    );
    let t = tenant(&pool, "kopi").await;
    let o = member(&pool, t, "Owner").await;
    let t2 = tenant(&pool, "lain").await;
    let o2 = member(&pool, t2, "Owner").await;
    publish_doc(&a, t, &o, studio_doc()).await;

    // Tanpa identitas shell / shell build lain → 404 (Caddy menyajikan shell statisnya; CSP tetap cocok).
    let plain = get(
        &router,
        "/api/v1/public/document?path=/",
        "kopi.ryasai.my.id",
    )
    .await;
    assert_eq!(plain.status, 404);
    let other = doc(
        &router,
        "/api/v1/public/document?path=/",
        "kopi.ryasai.my.id",
        "build lain",
    )
    .await;
    assert_eq!(other.status, 404);
    let r = doc(
        &router,
        "/api/v1/public/document?path=/",
        "kopi.ryasai.my.id",
        SHELL,
    )
    .await;
    assert_eq!(r.status, 200);
    let html = String::from_utf8(r.bytes.clone()).unwrap();
    assert_eq!(r.headers["content-type"], "text/html; charset=utf-8");
    // UAT 15 Nov perf #5: HTML toko = no-cache + ETag (publish langsung terlihat, revalidasi 304).
    assert_eq!(r.headers["cache-control"], "no-cache");
    let etag = r.headers["etag"].to_str().unwrap().to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    let again = doc_if(
        &router,
        "/api/v1/public/document?path=/",
        "kopi.ryasai.my.id",
        SHELL,
        Some(&etag),
    )
    .await;
    assert_eq!(again.status, 304);
    assert!(again.bytes.is_empty());
    assert_eq!(again.headers["etag"], etag.as_str());
    let stale = doc_if(
        &router,
        "/api/v1/public/document?path=/",
        "kopi.ryasai.my.id",
        SHELL,
        Some("\"lama\""),
    )
    .await;
    assert_eq!(stale.status, 200);
    // Judul SEO dari versi terbit, ter-escape; judul shell diganti; skrip shell utuh.
    assert!(
        html.contains("<title>Kopi Senja &lt;/title&gt;&lt;script&gt;x&lt;/script&gt;</title>"),
        "{html}"
    );
    assert_eq!(html.matches("<title>").count(), 1);
    assert!(html.contains("content=\"Kopi &amp; roti &quot;enak&quot;\""));
    assert!(html.contains("<link rel=\"canonical\" href=\"https://kopi.ryasai.my.id/\">"));
    assert!(html.contains("og:site_name\" content=\"Kopi &lt;Senja&gt;\""));
    assert!(html.contains("application/ld+json") && html.contains("LocalBusiness"));
    assert!(html.contains("<script src=\"/_next/static/a.js\"></script>"));
    assert!(!html.contains("<script>x"));

    // Path berbahaya → kanonik "/".
    let r = doc(
        &router,
        "/api/v1/public/document?path=/%22%3E%3Cscript%3E",
        "kopi.ryasai.my.id",
        SHELL,
    )
    .await;
    assert!(
        String::from_utf8(r.bytes)
            .unwrap()
            .contains("href=\"https://kopi.ryasai.my.id/\">")
    );

    // Tenant lain tanpa versi terbit: tidak membocorkan meta toko pertama.
    let _ = o2;
    let r = doc(
        &router,
        "/api/v1/public/document?path=/",
        "lain.ryasai.my.id",
        SHELL,
    )
    .await;
    let other = String::from_utf8(r.bytes).unwrap();
    assert!(!other.contains("Kopi"), "{other}");

    // Host tak dikenal → 404 (Caddy jatuh balik ke shell statis).
    assert_eq!(
        doc(
            &router,
            "/api/v1/public/document?path=/",
            "tidakada.ryasai.my.id",
            SHELL,
        )
        .await
        .status,
        404
    );

    // Fitur nonaktif (tanpa shell) → 404.
    assert_eq!(
        get(
            &a.router,
            "/api/v1/public/document?path=/",
            "kopi.ryasai.my.id"
        )
        .await
        .status,
        404
    );

    // robots & sitemap per Host.
    let r = get(&router, "/api/v1/public/robots.txt", "kopi.ryasai.my.id").await;
    assert_eq!(r.status, 200);
    assert!(
        String::from_utf8(r.bytes)
            .unwrap()
            .contains("Sitemap: https://kopi.ryasai.my.id/sitemap.xml")
    );
    let r = get(&router, "/api/v1/public/sitemap.xml", "kopi.ryasai.my.id").await;
    assert_eq!(r.status, 200);
    let xml = String::from_utf8(r.bytes).unwrap();
    assert!(
        xml.contains("<loc>https://kopi.ryasai.my.id/</loc>"),
        "{xml}"
    );
}

/// UAT 15 Nov customer #2: og:image tidak boleh signed URL yang kedaluwarsa — crawler WhatsApp/FB
/// menyimpan URL-nya. Head memakai URL stabil di host toko yang mengalihkan ke signed URL segar.
#[sqlx::test(migrator = "MIGRATOR")]
async fn og_image_is_a_stable_redirect_not_an_expiring_signed_url(pool: PgPool) {
    let a = app(&pool).await;
    let router = build_router(
        a.state.clone().with_site(SiteHead::new(
            Some(SHELL.into()),
            "https://api.ryasai.my.id".into(),
            "https".into(),
        )),
        LIMITS,
    );
    let t = tenant(&pool, "kopi").await;
    let o = member(&pool, t, "Owner").await;
    let r#ref = upload(&a.router, t, &o, png()).await.json()["data"]["ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut d = studio_doc();
    d["seo"]["image"] = json!(r#ref);
    publish_doc(&a, t, &o, d).await;

    let r = doc(
        &router,
        "/api/v1/public/document?path=/",
        "kopi.ryasai.my.id",
        SHELL,
    )
    .await;
    assert_eq!(r.status, 200);
    let html = String::from_utf8(r.bytes).unwrap();
    assert!(
        html.contains(
            "og:image\" content=\"https://kopi.ryasai.my.id/_api/public/og-image?path=/\""
        ),
        "{html}"
    );
    assert!(
        !html.contains("og:image\" content=\"https://api.ryasai.my.id/files/"),
        "{html}"
    );

    let g = get(
        &router,
        "/api/v1/public/og-image?path=/",
        "kopi.ryasai.my.id",
    )
    .await;
    assert_eq!(g.status, 302);
    let loc = g.headers["location"].to_str().unwrap();
    assert!(
        loc.starts_with("https://api.ryasai.my.id/files/") && loc.ends_with("&w=1280"),
        "{loc}"
    );
    assert_eq!(g.headers["cache-control"], "public, max-age=300");

    // Toko lain tanpa gambar / host tak dikenal → 404.
    let _ = tenant(&pool, "lain").await;
    assert_eq!(
        get(
            &router,
            "/api/v1/public/og-image?path=/",
            "lain.ryasai.my.id"
        )
        .await
        .status,
        404
    );
    assert_eq!(
        get(
            &router,
            "/api/v1/public/og-image?path=/",
            "tidakada.ryasai.my.id"
        )
        .await
        .status,
        404
    );
}

/// Blog/artikel: halaman artikel mendapat judul & deskripsi sendiri, og:type article, JSON-LD Article; sitemap
/// memuat semua halaman terbit dengan lastmod.
#[sqlx::test(migrator = "MIGRATOR")]
async fn article_page_has_its_own_seo_and_is_in_the_sitemap(pool: PgPool) {
    let a = app(&pool).await;
    let router = build_router(
        a.state.clone().with_site(SiteHead::new(
            Some(SHELL.into()),
            "https://api.ryasai.my.id".into(),
            "https".into(),
        )),
        LIMITS,
    );
    let t = tenant(&pool, "kopi").await;
    let o = member(&pool, t, "Owner").await;
    publish_doc(&a, t, &o, studio_doc()).await;
    let w = format!("/api/v1/tenants/{t}/website");
    let r = json(
        &a.router,
        Method::POST,
        &format!("{w}/pages"),
        Some(&o),
        Some(json!({"slug": "cara-seduh", "title": "Cara seduh kopi"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", r.json());
    let mut art = studio_doc();
    art["seo"] = json!({"description": "Panduan V60 untuk pemula"});
    art["article"] = json!({"excerpt": "Panduan singkat", "author": "Rina", "published_at": "2026-11-01", "tags": ["kopi", "v60"]});
    art["blocks"] = json!([{"id": "isi", "kind": "article_body", "x": 0, "y": 0, "w": 12, "h": 10, "style": "plain", "data": {"body": "## Alat\nSiapkan V60.", "toc": true}}]);
    let d = json(
        &a.router,
        Method::GET,
        &format!("{w}/pages/cara-seduh/draft"),
        Some(&o),
        None,
    )
    .await;
    let rev = d.json()["data"]["revision"].as_i64().unwrap();
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{w}/pages/cara-seduh/draft"),
        Some(&o),
        Some(json!({"content": art, "base_revision": rev})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());
    let r = json(
        &a.router,
        Method::POST,
        &format!("{w}/pages/cara-seduh/publish"),
        Some(&o),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());

    let html = String::from_utf8(
        doc(
            &router,
            "/api/v1/public/document?path=/cara-seduh",
            "kopi.ryasai.my.id",
            SHELL,
        )
        .await
        .bytes,
    )
    .unwrap();
    assert!(
        // Judul halaman artikel = "{headline} - {nama toko} - UMKM OS" (pemisah "-" dan sufiks
        // brand, sama dengan halaman website biasa; lihat build_meta di site_head.rs).
        html.contains("<title>Cara seduh kopi - Kopi &lt;Senja&gt; - UMKM OS</title>"),
        "{html}"
    );
    assert!(
        html.contains("content=\"Panduan V60 untuk pemula\""),
        "{html}"
    );
    assert!(
        html.contains("<meta property=\"og:type\" content=\"article\">"),
        "{html}"
    );
    assert!(html.contains("\"@type\":\"Article\""), "{html}");
    assert!(html.contains("\"datePublished\":\"2026-11-01\""), "{html}");
    assert!(html.contains("\"name\":\"Rina\""), "{html}");

    // Situs publik: daftar halaman membawa metadata artikel (plugin Daftar artikel).
    let site = get(&router, "/api/v1/public/site", "kopi.ryasai.my.id").await;
    let pages = site.json()["data"]["pages"].clone();
    let p = pages
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "cara-seduh")
        .unwrap()
        .clone();
    assert_eq!(p["article"]["author"], "Rina");

    let xml = String::from_utf8(
        get(&router, "/api/v1/public/sitemap.xml", "kopi.ryasai.my.id")
            .await
            .bytes,
    )
    .unwrap();
    assert!(
        xml.contains("<loc>https://kopi.ryasai.my.id/cara-seduh</loc><lastmod>"),
        "{xml}"
    );
}

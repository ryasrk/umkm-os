//! `<head>` situs toko per Host (ADR-0018, P9-5): judul, deskripsi, kanonik, Open Graph, favicon, dan
//! JSON-LD disisipkan ke shell statis hasil build frontend, supaya crawler pratinjau tautan (WhatsApp,
//! Facebook) yang TIDAK menjalankan JavaScript tetap melihat judul & gambar toko. Juga `robots.txt` dan
//! `sitemap.xml` per toko. Sumber data: versi TERBIT + profil bisnis (draf tidak pernah bocor).
//!
//! Keamanan: semua nilai di-escape untuk konteks HTML; JSON-LD adalah blok data (`application/ld+json`,
//! tidak dieksekusi) yang di-escape untuk konteks `<script>` (`<`, `>`, `&` → `\u003c` dst.), sehingga
//! teks pemilik toko tidak bisa menutup elemen atau menyuntik markup.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use umkm_infrastructure::{business_repo, catalog_repo, website_repo};
use umkm_shared::ids::TenantId;
use uuid::Uuid;

use crate::{
    AppState,
    business::{BusinessView, to_view},
    catalog::public_view,
    http_util::{internal, pool},
    middleware::tenant::HostTenant,
    website::signed_assets,
};

/// Konfigurasi proses: shell statis + origin publik API (URL absolut gambar) + skema situs.
#[derive(Debug, Clone, Default)]
pub struct SiteHead {
    /// Isi `site.html` hasil build frontend. `None` = fitur nonaktif (Caddy menyajikan file statis).
    pub shell: Option<String>,
    /// SHA-256 hex `shell`; harus sama dengan header `X-Web-Shell` dari Caddy (build web yang sedang disajikan).
    pub shell_hash: String,
    pub api_origin: String,
    pub scheme: String,
}

impl SiteHead {
    pub fn new(shell: Option<String>, api_origin: String, scheme: String) -> Self {
        let shell_hash = shell.as_deref().map(shell_hash).unwrap_or_default();
        Self {
            shell,
            shell_hash,
            api_origin,
            scheme,
        }
    }
}

/// Data `<head>` satu halaman (sudah final; belum di-escape).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeadMeta {
    pub title: String,
    pub description: String,
    pub url: String,
    pub site_name: String,
    pub og_type: &'static str,
    pub image: Option<String>,
    pub favicon: Option<String>,
    pub json_ld: Option<Value>,
    /// `<meta name="keywords">` — dipisahkan koma oleh pemilik toko (SEO klasik; juga JSON-LD).
    pub keywords: Option<String>,
}

/// Escape untuk teks & nilai atribut HTML (dikutip ganda).
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() && c != '\n' => {}
            c => o.push(c),
        }
    }
    o
}

/// JSON aman di dalam `<script type="application/ld+json">`.
pub fn json_for_script(v: &Value) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Potong ke `max` karakter (deskripsi meta), tanpa memotong di tengah karakter.
fn clip(s: &str, max: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        t
    } else {
        let mut c: String = t.chars().take(max.saturating_sub(1)).collect();
        c.push('…');
        c
    }
}

pub fn render_head(m: &HeadMeta) -> String {
    let mut h = String::new();
    h.push_str(&format!("<title>{}</title>", esc(&m.title)));
    if !m.description.is_empty() {
        h.push_str(&format!(
            "<meta name=\"description\" content=\"{}\">",
            esc(&m.description)
        ));
    }
    h.push_str(&format!(
        "<link rel=\"canonical\" href=\"{}\">",
        esc(&m.url)
    ));
    for (p, v) in [
        ("og:title", m.title.as_str()),
        ("og:description", m.description.as_str()),
        ("og:url", m.url.as_str()),
        ("og:site_name", m.site_name.as_str()),
        ("og:type", m.og_type),
        ("og:locale", "id_ID"),
    ] {
        if !v.is_empty() {
            h.push_str(&format!("<meta property=\"{p}\" content=\"{}\">", esc(v)));
        }
    }
    if let Some(kw) = m.keywords.as_deref().filter(|k| !k.trim().is_empty()) {
        h.push_str(&format!("<meta name=\"keywords\" content=\"{}\">", esc(kw)));
    }
    if let Some(img) = &m.image {
        h.push_str(&format!(
            "<meta property=\"og:image\" content=\"{}\">",
            esc(img)
        ));
        // Dimensi & tipe eksplisit: sebagian crawler (Discord, LinkedIn) butuh ini untuk
        // pratinjau gambar yang benar; WhatsApp cukup og:image + twitter:card.
        h.push_str("<meta property=\"og:image:width\" content=\"1280\">");
        h.push_str("<meta property=\"og:image:height\" content=\"630\">");
        h.push_str(&format!(
            "<meta property=\"og:image:alt\" content=\"{}\">",
            esc(&m.title)
        ));
        // Twitter Card lengkap (judul/deskripsi/gambar) — dipakai X & sebagian aplikasi lain.
        h.push_str(&format!(
            "<meta name=\"twitter:title\" content=\"{}\">",
            esc(&m.title)
        ));
        if !m.description.is_empty() {
            h.push_str(&format!(
                "<meta name=\"twitter:description\" content=\"{}\">",
                esc(&m.description)
            ));
        }
        h.push_str(&format!(
            "<meta name=\"twitter:image\" content=\"{}\">",
            esc(img)
        ));
        h.push_str("<meta name=\"twitter:card\" content=\"summary_large_image\">");
    } else {
        h.push_str(&format!(
            "<meta name=\"twitter:title\" content=\"{}\">",
            esc(&m.title)
        ));
        if !m.description.is_empty() {
            h.push_str(&format!(
                "<meta name=\"twitter:description\" content=\"{}\">",
                esc(&m.description)
            ));
        }
        h.push_str("<meta name=\"twitter:card\" content=\"summary\">");
    }
    if let Some(f) = &m.favicon {
        h.push_str(&format!("<link rel=\"icon\" href=\"{}\">", esc(f)));
    }
    if let Some(ld) = &m.json_ld {
        h.push_str(&format!(
            "<script type=\"application/ld+json\">{}</script>",
            json_for_script(ld)
        ));
    }
    h
}

/// Sisipkan `head` ke shell: buang `<title>` & meta description bawaan shell, lalu taruh sebelum `</head>`.
pub fn inject(shell: &str, head: &str) -> String {
    let mut s = shell.to_owned();
    if let (Some(a), Some(b)) = (s.find("<title>"), s.find("</title>"))
        && a < b
    {
        s.replace_range(a..b + "</title>".len(), "");
    }
    while let Some(a) = s.find("<meta name=\"description\"") {
        let Some(len) = s[a..].find('>') else { break };
        s.replace_range(a..a + len + 1, "");
    }
    match s.find("</head>") {
        Some(i) => {
            s.insert_str(i, head);
            s
        }
        None => format!("{head}{s}"),
    }
}

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// JSON-LD LocalBusiness dari profil bisnis (schema.org; Google Search Central rich result).
pub fn local_business(b: &BusinessView, url: &str, image: Option<&str>) -> Value {
    let p = &b.profile;
    let mut v = json!({"@context": "https://schema.org", "@type": "LocalBusiness", "name": p.name, "url": url});
    if let Some(d) = p.description.as_deref().filter(|d| !d.trim().is_empty()) {
        v["description"] = json!(clip(d, 300));
    }
    if let Some(t) = p
        .phone
        .as_deref()
        .or(p.whatsapp.as_deref())
        .filter(|t| !t.trim().is_empty())
    {
        v["telephone"] = json!(t);
    }
    if let Some(i) = image {
        v["image"] = json!(i);
    }
    if let Some(a) = b.addresses.first() {
        v["address"] =
            json!({"@type": "PostalAddress", "streetAddress": a.address, "addressCountry": "ID"});
        if let (Some(lat), Some(lng)) = (a.latitude, a.longitude) {
            v["geo"] = json!({"@type": "GeoCoordinates", "latitude": lat, "longitude": lng});
        }
    }
    let hours: Vec<Value> = b
        .hours
        .iter()
        .filter_map(|h| {
            let d = DAYS.get(usize::from(h.day_of_week).checked_sub(1)?)?;
            Some(json!({"@type": "OpeningHoursSpecification", "dayOfWeek": format!("https://schema.org/{d}"), "opens": h.open, "closes": h.close}))
        })
        .collect();
    if !hours.is_empty() {
        v["openingHoursSpecification"] = json!(hours);
    }
    v
}

// ------------------------------------------------------------------ handler

#[derive(Debug, Deserialize)]
pub struct DocQuery {
    path: Option<String>,
}

fn host_of(h: &HeaderMap) -> String {
    h.get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
                .collect()
        })
        .unwrap_or_default()
}

/// Path dokumen aman untuk URL kanonik: hanya `[A-Za-z0-9/_-]`, maks 200.
fn clean_path(p: Option<&str>) -> String {
    let p = p.unwrap_or("/");
    if !p.starts_with('/')
        || p.len() > 200
        || !p
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_'))
    {
        return "/".into();
    }
    p.into()
}

fn cached(body: String, ctype: &'static str) -> Response {
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Host"));
    r
}

/// Dokumen HTML toko: `no-cache` + ETag kuat (hash isi) → browser selalu merevalidasi, tetapi
/// hanya mengunduh ulang bila isinya berubah (304). Sebelumnya `public, max-age=60` bertabrakan
/// dengan `no-cache` dari Caddy dan tanpa ETag tidak bisa 304 (UAT 15 Nov perf #5).
fn html_document(body: String, req: &HeaderMap) -> Response {
    let digest = Sha256::digest(body.as_bytes());
    let etag = format!(
        "\"{}\"",
        digest[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let fresh = req
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        });
    let mut r = if fresh {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut r = (StatusCode::OK, body).into_response();
        r.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        r
    };
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::VARY, HeaderValue::from_static("Host"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    r
}

/// `GET /api/v1/public/document?path=/…` → shell HTML dengan `<head>` toko. Gagal/nonaktif → 404,
/// dan Caddy jatuh balik ke shell statis (situs tetap tampil, hanya tanpa meta).
pub async fn document(
    State(state): State<AppState>,
    HostTenant(t): HostTenant,
    headers: HeaderMap,
    Query(q): Query<DocQuery>,
) -> Response {
    let Some(shell) = state.site.shell.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Shell API harus build yang SAMA dengan yang disajikan Caddy: skrip inline-nya di-hash ke CSP build itu.
    // Beda versi (deploy bertahap, tag api ≠ web) → 404 → Caddy menyajikan shell statisnya (situs tetap hidup).
    let served = headers
        .get("x-web-shell")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if served != state.site.shell_hash {
        tracing::warn!(served, own = %state.site.shell_hash, "site shell API ≠ build web; <head> per toko dilewati");
        return StatusCode::NOT_FOUND.into_response();
    }
    let host = host_of(&headers);
    let path = clean_path(q.path.as_deref());
    match build_meta(&state, t, &host, &path).await {
        Ok(Some(mut meta)) => {
            stable_image(&mut meta, &site_origin(&state, &host), &path);
            let mut head = render_head(&meta);
            // Gambar toko dilayani origin API: buka koneksinya sejak awal (bukan setelah JS & data).
            if !state.site.api_origin.is_empty() {
                head.push_str(&format!(
                    "<link rel=\"preconnect\" href=\"{}\">",
                    esc(&state.site.api_origin)
                ));
            }
            if let Some(data) = site_data(&state, t, &path).await {
                head.push_str(&data);
            }
            html_document(inject(&shell, &head), &headers)
        }
        // Langganan habis / toko tanpa versi terbit: shell apa adanya.
        Ok(None) => html_document(shell, &headers),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Slug halaman website untuk `path` (sama dengan frontend `pageSlug`), atau `None` untuk rute toko
/// (/produk/…, /keranjang, /akun, …) yang memuat datanya sendiri.
fn page_slug_of(path: &str) -> Option<String> {
    let s = path.trim_matches('/');
    if s.is_empty() || s == "site" || s == "site.html" {
        return Some(website_repo::HOME.into());
    }
    let shop = matches!(
        s,
        "keranjang" | "pesanan" | "akun" | "kebijakan-privasi" | "syarat-transaksi"
    );
    (!shop && umkm_domain::website::is_valid_page_slug(s)).then(|| s.to_owned())
}

/// Batas produk yang ditampilkan halaman: limit terbesar dari blok/section `product_grid`
/// (bawaan 8, 1–24 — sama dengan frontend). `None` = halaman tanpa galeri produk.
fn grid_limit(content: &Value) -> Option<i64> {
    let blocks = content.get("blocks").and_then(Value::as_array);
    let sections = content.get("sections").and_then(Value::as_array);
    blocks
        .into_iter()
        .flatten()
        .filter(|b| b.get("kind").and_then(Value::as_str) == Some("product_grid"))
        .chain(
            sections
                .into_iter()
                .flatten()
                .filter(|s| s.get("type").and_then(Value::as_str) == Some("product_grid")),
        )
        .map(|b| {
            b.pointer("/data/limit")
                .and_then(Value::as_i64)
                .unwrap_or(8)
                .clamp(1, 24)
        })
        .max()
}

/// Data situs (isi `GET /public/site` + produk galeri) ditanam di dokumen sebagai
/// `<script type="application/json" id="__site">` — browser tidak perlu dua permintaan berantai
/// setelah JS termuat. Blok data JSON tidak dieksekusi, jadi CSP (hash skrip) tidak terpengaruh.
/// Gagal di sini tidak fatal: frontend jatuh balik ke fetch biasa.
///
/// Rute toko (produk, keranjang, akun, …) menanam situs beranda (nama, logo, warna & font toko untuk
/// tampilan halaman toko) dan — untuk `/produk/{slug}` — produknya sendiri.
async fn site_data(state: &AppState, t: TenantId, path: &str) -> Option<String> {
    let page = page_slug_of(path);
    // Halaman sistem yang sudah ditata di Studio dan terbit: tanam dokumennya (frontend merender blok
    // intinya). Belum terbit → situs beranda seperti rute toko lain (tampilan bawaan).
    let system = path.trim_matches('/').to_owned();
    let system = umkm_domain::website::is_system_slug(&system).then_some(system);
    let mut site = None;
    if let Some(s) = &system {
        site = crate::website::load_public_site(state, t, s.clone())
            .await
            .ok();
    }
    let site = match site {
        Some(x) => x,
        None => crate::website::load_public_site(
            state,
            t,
            page.clone().unwrap_or_else(|| website_repo::HOME.into()),
        )
        .await
        .ok()?,
    };
    let site = serde_json::to_value(site).ok()?;
    let products = match page
        .as_ref()
        .or(system.as_ref())
        .and_then(|_| site.pointer("/page/content"))
        .and_then(grid_limit)
    {
        Some(limit) => Some(
            crate::catalog::load_public_products(state, t, None, limit, 0)
                .await
                .ok()?,
        ),
        None => None,
    };
    let product = match path
        .strip_prefix("/produk/")
        .filter(|s| umkm_domain::catalog::is_valid_slug(s))
    {
        Some(slug) => crate::catalog::load_public_product(state, t, slug)
            .await
            .ok()
            .flatten()
            .and_then(|p| serde_json::to_value(p).ok()),
        None => None,
    };
    let data = json!({ "site": site, "products": products, "product": product });
    Some(format!(
        "<script type=\"application/json\" id=\"__site\">{}</script>",
        json_for_script(&data)
    ))
}

fn site_origin(state: &AppState, host: &str) -> String {
    let scheme = if state.site.scheme.is_empty() {
        "https"
    } else {
        state.site.scheme.as_str()
    };
    format!("{scheme}://{host}")
}

/// og:image & JSON-LD memakai URL STABIL di host toko, bukan signed URL `/files/…` yang kedaluwarsa
/// dalam ±1 jam: WhatsApp/Facebook menyimpan URL og:image dan mengambilnya lagi belakangan → 404
/// (UAT 15 Nov customer #2). `/_api/public/og-image` mengalihkan ke signed URL yang segar.
fn stable_image(meta: &mut HeadMeta, origin: &str, path: &str) {
    if meta.image.is_none() {
        return;
    }
    let stable = format!("{origin}/_api/public/og-image?path={path}");
    if let Some(ld) = meta.json_ld.as_mut()
        && ld.get("image").is_some()
    {
        ld["image"] = json!(stable);
    }
    meta.image = Some(stable);
}

/// `GET /api/v1/public/og-image?path=/…` → 302 ke signed URL gambar pratinjau halaman itu (varian
/// 1280 px). Pengalihan di-cache 5 menit — jauh di bawah sisa umur signature (≥ TTL/2).
pub async fn og_image(
    State(state): State<AppState>,
    HostTenant(t): HostTenant,
    headers: HeaderMap,
    Query(q): Query<DocQuery>,
) -> Response {
    let path = clean_path(q.path.as_deref());
    let image = match build_meta(&state, t, &host_of(&headers), &path).await {
        Ok(Some(m)) => m.image,
        _ => None,
    };
    let Some(mut url) = image else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if url.contains("/files/") && url.contains('?') {
        url.push_str("&w=1280");
    }
    let Ok(loc) = HeaderValue::from_str(&url) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut r = StatusCode::FOUND.into_response();
    let h = r.headers_mut();
    h.insert(header::LOCATION, loc);
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Host"));
    r
}

async fn build_meta(
    state: &AppState,
    t: TenantId,
    host: &str,
    path: &str,
) -> Result<Option<HeadMeta>, crate::ApiError> {
    if crate::billing::require_operational(state, t).await.is_err() {
        return Ok(None);
    }
    let p = pool(state)?;
    let Some(info) = business_repo::load(p, t).await.map_err(internal)? else {
        return Ok(None);
    };
    let biz = to_view(info);
    let content = website_repo::published_content(p, t, website_repo::HOME)
        .await
        .map_err(internal)?
        .map(|(_, c)| c);
    // Halaman selain beranda: SEO & metadata artikel dari halaman itu sendiri (jatuh ke beranda bila kosong).
    let page_slug = page_slug_of(path).filter(|s| s != website_repo::HOME);
    let page = match &page_slug {
        Some(s) => website_repo::published_content(p, t, s)
            .await
            .map_err(internal)?
            .map(|(_, c)| c),
        None => None,
    };
    let seo = content
        .as_ref()
        .and_then(|c| c.get("seo"))
        .cloned()
        .unwrap_or(Value::Null);
    let page_seo = page
        .as_ref()
        .and_then(|c| c.get("seo"))
        .cloned()
        .unwrap_or(Value::Null);
    let article = page
        .as_ref()
        .and_then(|c| c.get("article"))
        .filter(|a| a.is_object())
        .cloned();
    let pick = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let seo_str = |k: &str| pick(&seo, k);
    let page_str = |k: &str| pick(&page_seo, k);
    let art_str = |k: &str| article.as_ref().and_then(|a| pick(a, k));

    // Aset yang mungkin dipakai: gambar SEO, favicon, sampul/logo profil.
    let refs: Vec<String> = [
        page_str("image"),
        art_str("cover"),
        seo_str("image"),
        seo_str("favicon"),
        biz.profile.cover.clone(),
        biz.profile.logo.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let ids: BTreeSet<Uuid> = refs
        .iter()
        .filter_map(|r| r.strip_prefix("asset:"))
        .filter_map(|x| Uuid::parse_str(x).ok())
        .collect();
    let signed = signed_assets(state, t, ids).await?;
    let abs = |r: Option<String>| {
        r.and_then(|r| {
            signed
                .get(&r)
                .map(|p| format!("{}{p}", state.site.api_origin))
        })
    };

    let site_url = site_origin(state, host);
    let url = format!("{site_url}{path}");
    let name = biz.profile.name.clone();
    let store_image = abs(seo_str("image"))
        .or_else(|| abs(biz.profile.cover.clone()))
        .or_else(|| abs(biz.profile.logo.clone()));
    let favicon = abs(seo_str("favicon")).or_else(|| abs(biz.profile.logo.clone()));
    let store_desc = seo_str("description")
        .or_else(|| biz.profile.description.clone())
        .unwrap_or_default();

    // Halaman produk: judul/gambar/harga produk (pratinjau tautan produk di WhatsApp).
    if let Some(slug) = path
        .strip_prefix("/produk/")
        .filter(|s| umkm_domain::catalog::is_valid_slug(s))
        && let Some(prod) = catalog_repo::public_product(p, t, slug)
            .await
            .map_err(internal)?
    {
        let v = public_view(state, t, vec![prod]).await?.remove(0);
        let image = v
            .images
            .first()
            .map(|i| format!("{}{i}", state.site.api_origin))
            .or(store_image);
        let price = v
            .variants
            .first()
            .map(|x| x.promo_price.clone().unwrap_or_else(|| x.price.clone()));
        let mut ld = json!({"@context": "https://schema.org", "@type": "Product", "name": v.name, "url": url});
        if let Some(d) = v.description.as_deref() {
            ld["description"] = json!(clip(d, 300));
        }
        if let Some(i) = &image {
            ld["image"] = json!(i);
        }
        if let Some(pr) = &price {
            ld["offers"] = json!({"@type": "Offer", "price": pr, "priceCurrency": "IDR",
                "availability": if v.available { "https://schema.org/InStock" } else { "https://schema.org/OutOfStock" }});
        }
        // Keywords: meta halaman (v1) → toko → nama produk sebagai kata kunci terakhir.
        let product_kw = page_str("keywords")
            .or_else(|| seo_str("keywords"))
            .map(|k| format!("{k}, {}", v.name));
        if let Some(k) = product_kw.as_deref().filter(|k| !k.trim().is_empty()) {
            ld["keywords"] = json!(k);
        }
        return Ok(Some(HeadMeta {
            title: format!("{} - {name} - UMKM OS", v.name),
            description: clip(v.description.as_deref().unwrap_or(&store_desc), 160),
            url,
            site_name: name,
            og_type: "product",
            image,
            favicon,
            json_ld: Some(ld),
            keywords: product_kw,
        }));
    }

    // Halaman lain (Tentang, Blog, artikel …): judul & deskripsi halaman itu; artikel → JSON-LD Article.
    if page.is_some()
        && let Some(slug) = &page_slug
    {
        let page_title = website_repo::published_pages(p, t)
            .await
            .map_err(internal)?
            .into_iter()
            .find(|x| &x.slug == slug)
            .map(|x| x.title)
            .unwrap_or_else(|| slug.clone());
        let headline = page_str("title").unwrap_or_else(|| page_title.clone());
        let desc = page_str("description")
            .or_else(|| art_str("excerpt"))
            .unwrap_or_else(|| store_desc.clone());
        let image = abs(page_str("image"))
            .or_else(|| abs(art_str("cover")))
            .or(store_image.clone());
        let (og_type, ld) = if article.is_some() {
            let mut ld = json!({"@context": "https://schema.org", "@type": "Article", "headline": clip(&headline, 110),
                "url": url, "mainEntityOfPage": url,
                "publisher": {"@type": "Organization", "name": name}});
            if !desc.is_empty() {
                ld["description"] = json!(clip(&desc, 300));
            }
            if let Some(i) = &image {
                ld["image"] = json!(i);
            }
            if let Some(d) = art_str("published_at") {
                ld["datePublished"] = json!(d);
            }
            ld["author"] = match art_str("author") {
                Some(a) => json!({"@type": "Person", "name": a}),
                None => json!({"@type": "Organization", "name": name}),
            };
            if let Some(tags) = article
                .as_ref()
                .and_then(|a| a.get("tags"))
                .and_then(Value::as_array)
            {
                ld["keywords"] = json!(
                    tags.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            ("article", ld)
        } else {
            (
                "website",
                local_business(&biz, &site_url, store_image.as_deref()),
            )
        };
        // Keywords halaman → tag artikel → keywords toko.
        let page_kw = page_str("keywords")
            .or_else(|| {
                article
                    .as_ref()
                    .and_then(|a| a.get("tags"))
                    .and_then(Value::as_array)
                    .map(|tags| {
                        tags.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
            })
            .or_else(|| seo_str("keywords"));
        return Ok(Some(HeadMeta {
            title: format!("{headline} - {name} - UMKM OS"),
            description: clip(&desc, 160),
            url,
            site_name: name,
            og_type,
            image,
            favicon,
            json_ld: Some(ld),
            keywords: page_kw,
        }));
    }

    let title = seo_str("title").unwrap_or_else(|| format!("{name} - UMKM OS"));
    let home_kw = seo_str("keywords");
    let mut ld = local_business(&biz, &site_url, store_image.as_deref());
    if let Some(k) = home_kw.as_deref().filter(|k| !k.trim().is_empty()) {
        ld["keywords"] = json!(k);
    }
    Ok(Some(HeadMeta {
        json_ld: Some(ld),
        title,
        description: clip(&store_desc, 160),
        url,
        site_name: name,
        og_type: "website",
        image: store_image,
        favicon,
        keywords: home_kw,
    }))
}

/// `GET /api/v1/public/robots.txt` — izinkan semua, tunjuk sitemap toko.
pub async fn robots(
    HostTenant(_t): HostTenant,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let scheme = if state.site.scheme.is_empty() {
        "https"
    } else {
        state.site.scheme.as_str()
    };
    let body = format!(
        "User-agent: *\nAllow: /\nDisallow: /keranjang\nDisallow: /akun\nDisallow: /pesanan\nSitemap: {scheme}://{}/sitemap.xml\n",
        host_of(&headers)
    );
    cached(body, "text/plain; charset=utf-8")
}

/// `GET /api/v1/public/sitemap.xml` — semua halaman terbit (dengan `lastmod`) + produk aktif (maks 500).
pub async fn sitemap(
    State(state): State<AppState>,
    HostTenant(t): HostTenant,
    headers: HeaderMap,
) -> Response {
    let scheme = if state.site.scheme.is_empty() {
        "https"
    } else {
        state.site.scheme.as_str()
    };
    let base = format!("{scheme}://{}", host_of(&headers));
    // (url, lastmod) — halaman terbit (beranda, halaman lain, artikel) lalu produk aktif.
    let mut urls: Vec<(String, Option<String>)> = Vec::new();
    if crate::billing::require_operational(&state, t).await.is_ok()
        && let Ok(p) = pool(&state)
    {
        for pg in website_repo::published_pages(p, t)
            .await
            .unwrap_or_default()
        {
            let path = if pg.slug == website_repo::HOME {
                "/".to_owned()
            } else {
                format!("/{}", pg.slug)
            };
            urls.push((format!("{base}{path}"), Some(pg.published_at)));
        }
        if let Ok((list, _)) = catalog_repo::public_products(p, t, None, 500, 0).await {
            urls.extend(
                list.into_iter()
                    .map(|x| (format!("{base}/produk/{}", x.slug), None)),
            );
        }
    }
    if urls.is_empty() {
        urls.push((format!("{base}/"), None));
    }
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for (u, lastmod) in urls {
        match lastmod {
            Some(d) => xml.push_str(&format!(
                "  <url><loc>{}</loc><lastmod>{}</lastmod></url>\n",
                esc(&u),
                esc(&d)
            )),
            None => xml.push_str(&format!("  <url><loc>{}</loc></url>\n", esc(&u))),
        }
    }
    xml.push_str("</urlset>\n");
    cached(xml, "application/xml; charset=utf-8")
}

/// SHA-256 hex isi shell (sama dengan `frontend/scripts/csp.mjs`).
pub fn shell_hash(shell: &str) -> String {
    Sha256::digest(shell.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/public/document", get(document))
        .route("/api/v1/public/og-image", get(og_image))
        .route("/api/v1/public/robots.txt", get(robots))
        .route("/api/v1/public/sitemap.xml", get(sitemap))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> HeadMeta {
        HeadMeta {
            title: "Kopi \"Senja\" </title><script>alert(1)</script>".into(),
            description: "Kedai <b>kopi</b> & roti".into(),
            url: "https://kopisenja.example/".into(),
            site_name: "Kopi Senja".into(),
            og_type: "website",
            image: Some("https://api.example/f/1?sig=a&b=c".into()),
            favicon: None,
            json_ld: Some(json!({"name": "</script><script>alert(1)</script>"})),
            keywords: Some("kopi, kedai <b>kopi</b> & roti".into()),
        }
    }

    #[test]
    fn head_escapes_text_attributes_and_json_ld() {
        let h = render_head(&meta());
        assert!(h.contains("<title>Kopi &quot;Senja&quot; &lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;</title>"));
        assert!(h.contains("content=\"Kedai &lt;b&gt;kopi&lt;/b&gt; &amp; roti\""));
        assert!(h.contains("og:image\" content=\"https://api.example/f/1?sig=a&amp;b=c\""));
        // JSON-LD tidak bisa menutup <script>: hanya satu </script> (penutup blok sendiri).
        assert_eq!(h.matches("</script>").count(), 1);
        assert!(h.contains("\\u003c/script\\u003e"));
        assert!(!h.contains("<script>alert"));
        assert!(h.contains("twitter:card\" content=\"summary_large_image\""));
        // Keywords meta di-escape seperti atribut lain; OG/Twitter lengkap untuk Discord/X.
        assert!(h.contains(
            "name=\"keywords\" content=\"kopi, kedai &lt;b&gt;kopi&lt;/b&gt; &amp; roti\""
        ));
        assert!(h.contains("og:image:alt\" content=\"Kopi &quot;Senja&quot; &lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;\""));
        assert!(h.contains("og:image:width\" content=\"1280\""));
        assert!(h.contains("twitter:image\" content=\"https://api.example/f/1?sig=a&amp;b=c\""));
        assert!(h.contains("twitter:title\" content=\"Kopi &quot;Senja&quot; &lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;\""));
    }

    #[test]
    fn inject_replaces_shell_title_and_description() {
        let shell = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>UMKM OS</title><meta name=\"description\" content=\"x\"/><script src=\"/_next/a.js\"></script></head><body></body></html>";
        let out = inject(shell, &render_head(&meta()));
        assert_eq!(out.matches("<title>").count(), 1);
        assert!(!out.contains("<title>UMKM OS</title>"));
        assert_eq!(out.matches("name=\"description\"").count(), 1);
        assert!(out.find("og:title").unwrap() < out.find("</head>").unwrap());
        assert!(
            out.contains("<script src=\"/_next/a.js\"></script>"),
            "skrip shell tetap utuh"
        );
    }

    #[test]
    fn clean_path_and_host_reject_injection() {
        assert_eq!(clean_path(Some("/produk/kopi-susu")), "/produk/kopi-susu");
        assert_eq!(clean_path(Some("/\"><script>")), "/");
        assert_eq!(clean_path(Some("https://evil")), "/");
        assert_eq!(clean_path(None), "/");
        let mut h = HeaderMap::new();
        h.insert(
            header::HOST,
            HeaderValue::from_static("kopi.ryasai.my.id\"><x"),
        );
        assert_eq!(host_of(&h), "kopi.ryasai.my.idx");
    }

    #[test]
    fn clip_and_esc_edges() {
        assert_eq!(clip("a  b\nc", 10), "a b c");
        assert_eq!(clip(&"x".repeat(200), 160).chars().count(), 160);
        assert_eq!(esc("a\u{0}b'"), "ab&#39;");
    }

    #[test]
    fn page_slug_and_grid_limit() {
        assert_eq!(page_slug_of("/").as_deref(), Some("home"));
        assert_eq!(page_slug_of("/menu").as_deref(), Some("menu"));
        assert_eq!(page_slug_of("/keranjang"), None);
        assert_eq!(page_slug_of("/produk/kopi"), None);
        assert_eq!(page_slug_of("/Bad!"), None);
        let v2 = json!({ "blocks": [
            { "kind": "product_grid", "data": { "limit": 6 } },
            { "kind": "product_grid", "data": {} },
            { "kind": "hero", "data": { "limit": 99 } }
        ] });
        assert_eq!(grid_limit(&v2), Some(8));
        assert_eq!(
            grid_limit(
                &json!({ "sections": [{ "type": "product_grid", "data": { "limit": 99 } }] })
            ),
            Some(24)
        );
        assert_eq!(
            grid_limit(&json!({ "blocks": [{ "kind": "hero", "data": {} }] })),
            None
        );
    }
}

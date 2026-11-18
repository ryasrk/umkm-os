//! P2-5: business profile/alamat/jam (AC-BIZ-001..007), role aplikasi + RLS.

mod common;

use axum::http::Method;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use umkm_database::MIGRATOR;
use umkm_domain::business::OpenStatus;
use umkm_infrastructure::business_repo;
use umkm_shared::ids::TenantId;
use uuid::Uuid;

fn base(t: Uuid) -> String {
    format!("/api/v1/tenants/{t}/business")
}

fn profile() -> Value {
    json!({
        "name": "Warung Bu Sri", "description": "Masakan rumahan", "phone": "+62 812-3456-7890",
        "email": "Halo@Warung.id", "whatsapp": "081234567890",
        "socials": [{"platform": "instagram", "url": "https://instagram.com/warung"}],
        "timezone": "Asia/Jakarta", "pickup_available": true, "delivery_available": false,
        "fulfillment_note": "Ambil di kasir"
    })
}

fn error_paths(r: &Res) -> Vec<String> {
    r.json()["error"]["details"]["errors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    format!(
                        "{}:{}",
                        e["path"].as_str().unwrap(),
                        e["code"].as_str().unwrap()
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn owner_updates_profile_and_it_is_audited(pool: PgPool) {
    // AC-BIZ-001, AC-BIZ-007
    let a = app(&pool).await;
    let t = tenant(&pool, "toko-a").await;
    let o = member(&pool, t, "Owner").await;
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(t)),
        Some(&o),
        Some(profile()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());
    let p = &r.json()["data"]["profile"];
    assert_eq!(p["name"], "Warung Bu Sri");
    // Sejak migrasi 0042, `business_profiles.phone` menyimpan digit kanonik TANPA `+`
    // (`normalize_phone_id`), berbeda dari `customers.phone` yang tetap E.164 dengan `+`.
    assert_eq!(p["phone"], "6281234567890", "dinormalisasi tanpa +");
    assert_eq!(
        p["whatsapp"], "081234567890",
        "lokal dipertahankan apa adanya"
    );
    assert_eq!(p["email"], "halo@warung.id", "email di-lowercase");
    assert_eq!(p["pickup_available"], true);
    assert_eq!(p["fulfillment_note"], "Ambil di kasir");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE tenant_id = $1 AND action = 'business.updated'",
    )
    .bind(t)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    // Manager tidak punya tenant.update; Viewer boleh membaca
    let m = member(&pool, t, "Manager").await;
    let v = member(&pool, t, "Viewer").await;
    assert_eq!(
        json(
            &a.router,
            Method::PUT,
            &format!("{}/profile", base(t)),
            Some(&m),
            Some(profile())
        )
        .await
        .status,
        403
    );
    assert_eq!(
        json(
            &a.router,
            Method::GET,
            &format!("{}/profile", base(t)),
            Some(&v),
            None
        )
        .await
        .status,
        200
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn invalid_profile_fields_are_rejected_with_paths(pool: PgPool) {
    // AC-BIZ-002
    let a = app(&pool).await;
    let t = tenant(&pool, "toko-a").await;
    let o = member(&pool, t, "Owner").await;
    let mut p = profile();
    p["email"] = json!("bukan-email");
    p["phone"] = json!("123");
    p["socials"] = json!([{"platform": "instagram", "url": "javascript:alert(1)"}]);
    p["logo"] = json!("https://evil/x.png");
    p["name"] = json!("");
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(t)),
        Some(&o),
        Some(p),
    )
    .await;
    assert_eq!(r.status, 422);
    let got = error_paths(&r);
    for want in [
        "email:invalid_email",
        "phone:invalid_phone",
        "socials[0].url:unsafe_url",
        "logo:invalid_asset_ref",
        "name:required",
    ] {
        assert!(got.contains(&want.to_owned()), "{want}: {got:?}");
    }
    // Field tak dikenal ditolak (deny_unknown_fields)
    let mut p = profile();
    p["html"] = json!("<b>");
    assert_eq!(
        json(
            &a.router,
            Method::PUT,
            &format!("{}/profile", base(t)),
            Some(&o),
            Some(p)
        )
        .await
        .status,
        422
    );
    // Timezone tidak dikenal (dicek ke katalog PostgreSQL)
    let mut p = profile();
    p["timezone"] = json!("Mars/Olympus");
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(t)),
        Some(&o),
        Some(p),
    )
    .await;
    assert_eq!(error_paths(&r), vec!["timezone:invalid_timezone"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn logo_must_be_an_object_of_the_same_tenant(pool: PgPool) {
    let a = app(&pool).await;
    let ta = tenant(&pool, "toko-a").await;
    let tb = tenant(&pool, "toko-b").await;
    let oa = member(&pool, ta, "Owner").await;
    let ob = member(&pool, tb, "Owner").await;
    let own: String = upload(&a.router, ta, &oa, png()).await.json()["data"]["ref"]
        .as_str()
        .unwrap()
        .into();
    let foreign: String = upload(&a.router, tb, &ob, png()).await.json()["data"]["ref"]
        .as_str()
        .unwrap()
        .into();
    let mut p = profile();
    p["logo"] = json!(foreign);
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(ta)),
        Some(&oa),
        Some(p),
    )
    .await;
    assert_eq!(error_paths(&r), vec!["logo:invalid_asset_ref"]);
    let mut p = profile();
    p["logo"] = json!(own.clone());
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(ta)),
        Some(&oa),
        Some(p),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["data"]["profile"]["logo"], own);
    // Logo yang dipakai tidak dapat dihapus (FK RESTRICT -> 409)
    let id = own.trim_start_matches("asset:");
    let r = json(
        &a.router,
        Method::DELETE,
        &format!("/api/v1/tenants/{ta}/uploads/{id}"),
        Some(&oa),
        None,
    )
    .await;
    assert_eq!(r.status, 409);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn addresses_validate_coordinates(pool: PgPool) {
    // AC-BIZ-003
    let a = app(&pool).await;
    let t = tenant(&pool, "toko-a").await;
    let o = member(&pool, t, "Owner").await;
    let url = format!("{}/addresses", base(t));
    let bad = json!([
        {"address": "Jl. A", "latitude": 91.0, "longitude": 0.0},
        {"address": "Jl. B", "latitude": 1.0},
        {"address": " "}
    ]);
    let r = json(&a.router, Method::PUT, &url, Some(&o), Some(bad)).await;
    assert_eq!(
        error_paths(&r),
        vec![
            "[0].latitude:out_of_range",
            "[1]:latitude_longitude_pair",
            "[2].address:required"
        ]
    );
    let ok = json!([
        {"label": "Pusat", "address": "Jl. Merdeka 1", "latitude": -6.2, "longitude": 106.8},
        {"address": "Cabang tanpa koordinat"}
    ]);
    let r = json(&a.router, Method::PUT, &url, Some(&o), Some(ok)).await;
    assert_eq!(r.status, 200, "{}", r.json());
    let list = r.json()["data"]["addresses"].clone();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["label"], "Pusat");
    assert_eq!(list[1]["latitude"], Value::Null);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn hours_reject_equal_duplicate_and_bad_values(pool: PgPool) {
    // AC-BIZ-004
    let a = app(&pool).await;
    let t = tenant(&pool, "toko-a").await;
    let o = member(&pool, t, "Owner").await;
    let url = format!("{}/hours", base(t));
    let bad = json!([
        {"day_of_week": 1, "open": "08:00", "close": "08:00"},
        {"day_of_week": 1, "open": "09:00", "close": "17:00"},
        {"day_of_week": 8, "open": "09:00", "close": "17:00"},
        {"day_of_week": 2, "open": "25:00", "close": "17:00"}
    ]);
    let r = json(&a.router, Method::PUT, &url, Some(&o), Some(bad)).await;
    assert_eq!(
        error_paths(&r),
        vec![
            "[0]:open_equals_close",
            "[1].day_of_week:duplicate_day",
            "[2].day_of_week:out_of_range",
            "[3].open:invalid_time"
        ]
    );
    let ok = json!([
        {"day_of_week": 1, "open": "08:00", "close": "17:00"},
        {"day_of_week": 6, "open": "18:00", "close": "02:00"}
    ]);
    let r = json(&a.router, Method::PUT, &url, Some(&o), Some(ok)).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json()["data"]["hours"][1]["close"],
        "02:00",
        "overnight diterima"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn exceptions_validate_and_reject_duplicates(pool: PgPool) {
    let a = app(&pool).await;
    let t = tenant(&pool, "toko-a").await;
    let o = member(&pool, t, "Owner").await;
    let url = format!("{}/hour-exceptions", base(t));
    let r = json(
        &a.router,
        Method::PUT,
        &url,
        Some(&o),
        Some(json!([
            {"date": "2026-13-01", "is_closed": true},
        ])),
    )
    .await;
    assert_eq!(
        error_paths(&r),
        vec!["exceptions:invalid_or_duplicate_date"],
        "bulan 13 ditolak DB"
    );
    let r = json(
        &a.router,
        Method::PUT,
        &url,
        Some(&o),
        Some(json!([
            {"date": "2026-12-25", "is_closed": true, "open": "08:00", "close": "10:00"},
            {"date": "26-12-25", "is_closed": true}
        ])),
    )
    .await;
    assert_eq!(
        error_paths(&r),
        vec!["[0]:closed_xor_hours", "[1].date:invalid_date"]
    );
    let r = json(
        &a.router,
        Method::PUT,
        &url,
        Some(&o),
        Some(json!([
            {"date": "2026-12-25", "is_closed": true}, {"date": "2026-12-25", "is_closed": true}
        ])),
    )
    .await;
    assert_eq!(r.status, 422);
    let r = json(
        &a.router,
        Method::PUT,
        &url,
        Some(&o),
        Some(json!([
            {"date": "2026-12-25", "is_closed": true, "note": "Natal"},
            {"date": "2026-12-31", "is_closed": false, "open": "08:00", "close": "12:00"}
        ])),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn status_follows_tenant_timezone_and_exceptions(pool: PgPool) {
    // AC-BIZ-005, AC-BIZ-006 — instan UTC tetap (tanpa jam sistem).
    let a = app(&pool).await;
    let t = tenant(&pool, "toko-a").await;
    let o = member(&pool, t, "Owner").await;
    // 2026-11-11 = Rabu (isodow 3)
    json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(t)),
        Some(&o),
        Some(profile()),
    )
    .await;
    let r = json(
        &a.router,
        Method::PUT,
        &format!("{}/hours", base(t)),
        Some(&o),
        Some(json!([
            {"day_of_week": 3, "open": "08:00", "close": "17:00"}
        ])),
    )
    .await;
    assert_eq!(r.status, 200);
    let app_db = app_pool(&pool).await;
    let tid = TenantId::from_uuid(t);
    let at = |utc: &'static str| {
        let p = app_db.clone();
        async move {
            business_repo::status(&p, tid, Some(utc))
                .await
                .unwrap()
                .unwrap()
        }
    };
    let s = at("2026-11-11T09:30:00Z").await; // 16:30 WIB
    assert_eq!(s.status, OpenStatus::Open);
    assert_eq!(s.local_time, "2026-11-11T16:30");
    assert_eq!(at("2026-11-11T10:30:00Z").await.status, OpenStatus::Closed); // 17:30 WIB
    assert_eq!(at("2026-11-11T00:30:00Z").await.status, OpenStatus::Closed); // 07:30 WIB
    // Ganti timezone ke Asia/Jayapura (UTC+9): 09:30Z = 18:30 -> tutup
    let mut p = profile();
    p["timezone"] = json!("Asia/Jayapura");
    json(
        &a.router,
        Method::PUT,
        &format!("{}/profile", base(t)),
        Some(&o),
        Some(p),
    )
    .await;
    assert_eq!(at("2026-11-11T09:30:00Z").await.status, OpenStatus::Closed);
    assert_eq!(at("2026-11-11T02:00:00Z").await.status, OpenStatus::Open); // 11:00 WIT
    // Exception libur pada tanggal lokal itu
    json(
        &a.router,
        Method::PUT,
        &format!("{}/hour-exceptions", base(t)),
        Some(&o),
        Some(json!([
            {"date": "2026-11-11", "is_closed": true}
        ])),
    )
    .await;
    assert_eq!(at("2026-11-11T02:00:00Z").await.status, OpenStatus::Closed);
    // Endpoint HTTP status tersedia
    let r = json(
        &a.router,
        Method::GET,
        &format!("{}/status", base(t)),
        Some(&o),
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["data"]["timezone"], "Asia/Jayapura");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn tenant_b_cannot_read_or_write_tenant_a_business(pool: PgPool) {
    let a = app(&pool).await;
    let ta = tenant(&pool, "toko-a").await;
    let tb = tenant(&pool, "toko-b").await;
    let oa = member(&pool, ta, "Owner").await;
    let ob = member(&pool, tb, "Owner").await;
    json(
        &a.router,
        Method::PUT,
        &format!("{}/addresses", base(ta)),
        Some(&oa),
        Some(json!([{"address": "Rahasia A"}])),
    )
    .await;
    for (m, path, body) in [
        (Method::GET, "profile", None),
        (Method::PUT, "profile", Some(profile())),
        (Method::PUT, "addresses", Some(json!([]))),
        (Method::GET, "status", None),
    ] {
        let r = json(
            &a.router,
            m,
            &format!("{}/{path}", base(ta)),
            Some(&ob),
            body,
        )
        .await;
        assert_eq!(r.status, 404, "{path}");
    }
    let r = json(
        &a.router,
        Method::GET,
        &format!("{}/profile", base(tb)),
        Some(&ob),
        None,
    )
    .await;
    assert_eq!(r.json()["data"]["addresses"].as_array().unwrap().len(), 0);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM business_addresses WHERE tenant_id = $1")
        .bind(ta)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "alamat A utuh");
}

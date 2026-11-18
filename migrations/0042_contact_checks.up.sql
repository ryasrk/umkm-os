-- P1-7 (lanjutan): CHECK constraint untuk kontak — DB jadi baris pertahanan terakhir.
--
-- Aturan ini WAJIB identik dengan:
--   - Rust  : crates/domain/src/business.rs → normalize_phone_id / is_valid_email
--   - TS    : frontend/src/lib/contact.ts  → normalizePhoneId / isValidEmail
-- Kalau ketiganya berbeda, pengguna mendapat pesan "tidak valid" padahal data ditolak
-- di tempat lain (atau sebaliknya) — bug yang mahal untuk ditacak.
--
-- Telepon: hanya angka, diawali `0` (lokal) atau `62` (internasional), 9–15 digit.
-- Email : satu `@`, lokal & domain tak kosong, domain bertitik, TLD ≥ 2 huruf.
--
-- CATATAN FORMAT PENYIMPANAN (penting — jangan samakan dengan format input):
--   business_profiles.phone / .whatsapp → digit kanonik TANPA `+`
--       (`normalize_phone_id` → "081234567890" / "6281234567890")
--   customers.phone / orders.customer_phone → E.164 DENGAN `+`
--       (`normalize_phone_e164` → "+6281234567890"; kolom ini menyimpan "+62…")
--   customers.email / customer_accounts.email → huruf kecil (lowercase)
--
-- Kolom NULL (tidak diisi) tetap boleh: CHECK di bawah hanya menolak nilai yang ADA
-- tetapi salah bentuk.

-- ---------------------------------------------------------------------------
-- business_profiles: telepon & WhatsApp (digit kanonik tanpa `+`)
-- ---------------------------------------------------------------------------
ALTER TABLE business_profiles
    DROP CONSTRAINT IF EXISTS ck_business_profiles_phone,
    DROP CONSTRAINT IF EXISTS ck_business_profiles_whatsapp,
    DROP CONSTRAINT IF EXISTS ck_business_profiles_email;

-- Hanya angka; awalan 0 atau 62; 9–15 digit.
ALTER TABLE business_profiles
    ADD CONSTRAINT ck_business_profiles_phone CHECK (
        phone IS NULL OR phone ~ '^(0|62)[0-9]{8,14}$'
    ),
    ADD CONSTRAINT ck_business_profiles_whatsapp CHECK (
        whatsapp IS NULL OR whatsapp ~ '^(0|62)[0-9]{8,14}$'
    ),
    ADD CONSTRAINT ck_business_profiles_email CHECK (
        email IS NULL OR (
            length(email) <= 254
            AND length(email) >= 3
            AND email = lower(btrim(email))
            AND email ~ '^[^@[:space:]@]+@[^@[:space:]]+(\.[^@[:space:]]+)+$'
            AND email ~ '\.[A-Za-z]{2,}$'
        )
    );

-- ---------------------------------------------------------------------------
-- customers: email (huruf kecil) & telepon (E.164 dengan `+`)
-- ---------------------------------------------------------------------------
ALTER TABLE customers
    DROP CONSTRAINT IF EXISTS ck_customers_email,
    DROP CONSTRAINT IF EXISTS ck_customers_phone;

ALTER TABLE customers
    ADD CONSTRAINT ck_customers_email CHECK (
        email IS NULL OR (
            length(email) <= 254
            AND length(email) >= 3
            AND email = lower(btrim(email))
            AND email ~ '^[^@[:space:]@]+@[^@[:space:]]+(\.[^@[:space:]]+)+$'
            AND email ~ '\.[A-Za-z]{2,}$'
        )
    ),
    -- E.164: "+62…" (tanpa "+" tidak sah — kolom ini diisi `normalize_phone_e164`).
    -- Panjang: `normalize_phone_e164` menerima 7–15 digit total termasuk "62" (lihat crm.rs),
    -- jadi setelah "+62" tersisa 5–13 digit. Angka ini SENGAJA lebih longgar daripada kolom
    -- `business_profiles` (9–15 digit) karena keduanya memakai normalizer berbeda.
    ADD CONSTRAINT ck_customers_phone CHECK (
        phone IS NULL OR phone ~ '^\+62[0-9]{5,13}$'
    );

-- ---------------------------------------------------------------------------
-- customer_accounts: email (huruf kecil)
-- ---------------------------------------------------------------------------
ALTER TABLE customer_accounts
    DROP CONSTRAINT IF EXISTS ck_customer_accounts_email;

ALTER TABLE customer_accounts
    ADD CONSTRAINT ck_customer_accounts_email CHECK (
        length(email) BETWEEN 3 AND 254
        AND email = lower(btrim(email))
        AND email ~ '^[^@[:space:]@]+@[^@[:space:]]+(\.[^@[:space:]]+)+$'
        AND email ~ '\.[A-Za-z]{2,}$'
    );

-- ---------------------------------------------------------------------------
-- users & invitations: email
--
-- `Email::parse` kini memakai aturan baku (`business::is_valid_email`), jadi nilainya
-- sudah bergaransi bentuk ketat. CHECK lama (`LIKE '%_@_%'`) hanya menjamin "@ di tengah"
-- — sekarang disamakan supaya DB tidak menerima bentuk yang API tolak.
-- CATATAN: `users.email` disimpan apa adanya (bisa huruf besar); keunikan dihitung lewat
-- `lower(email)`. Jadi lowercase TIDAK dipaksa di sini — hanya bentuknya.
-- ---------------------------------------------------------------------------
ALTER TABLE users
    DROP CONSTRAINT IF EXISTS ck_users_email;

ALTER TABLE users
    ADD CONSTRAINT ck_users_email CHECK (
        length(email) BETWEEN 3 AND 254
        AND email = btrim(email)
        AND email ~ '^[^@[:space:]@]+@[^@[:space:]]+(\.[^@[:space:]]+)+$'
        AND email ~ '\.[A-Za-z]{2,}$'
    );

ALTER TABLE invitations
    DROP CONSTRAINT IF EXISTS ck_invitations_email;

ALTER TABLE invitations
    ADD CONSTRAINT ck_invitations_email CHECK (
        length(email) BETWEEN 3 AND 254
        AND email ~ '^[^@[:space:]@]+@[^@[:space:]]+(\.[^@[:space:]]+)+$'
        AND email ~ '\.[A-Za-z]{2,}$'
    );

-- ---------------------------------------------------------------------------
-- orders: snapshot kontak saat checkout
-- ---------------------------------------------------------------------------
ALTER TABLE orders
    DROP CONSTRAINT IF EXISTS ck_orders_customer_email,
    DROP CONSTRAINT IF EXISTS ck_orders_customer_phone;

ALTER TABLE orders
    ADD CONSTRAINT ck_orders_customer_email CHECK (
        customer_email IS NULL OR (
            length(customer_email) <= 254
            AND length(customer_email) >= 3
            AND customer_email = lower(btrim(customer_email))
            AND customer_email ~ '^[^@[:space:]@]+@[^@[:space:]]+(\.[^@[:space:]]+)+$'
            AND customer_email ~ '\.[A-Za-z]{2,}$'
        )
    ),
    ADD CONSTRAINT ck_orders_customer_phone CHECK (
        customer_phone IS NULL OR customer_phone ~ '^\+62[0-9]{5,13}$'
    );

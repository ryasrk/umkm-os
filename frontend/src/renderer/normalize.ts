// Normalisasi dokumen `schema_version = 2` (ADR-0017) sebelum masuk renderer.
//
// Satu-satunya jalur data ke `StudioSite`. Server (Rust, `crates/domain/src/website/schema_v2.rs`) tetap
// penjaga utama; fungsi ini memastikan renderer tidak pernah menerima bentuk yang membuatnya crash bila
// data datang dari localStorage lama, draf yang disunting tangan, atau versi terbit dari masa lalu.
//
// Aturan yang dijaga di sini SENGAJA identik dengan validator server (daftar plugin, gaya, tata letak,
// permukaan, skala teks, batas grid). Menambah nilai di sini tanpa menambahnya di Rust hanya akan
// menghasilkan publish yang ditolak, jadi keduanya dijaga `crates/domain/tests/frontend_contract.rs`.

import { safeId, type BlockV2, type PageV2 } from "@/studio/doc";
import { PLUGINS } from "@/studio/plugins";

/** Plugin resmi (ADR-0019) — sama dengan `PLUGIN_KINDS` di server. */
const KINDS = new Set(PLUGINS.map((p) => p.kind));
/** Varian tata letak yang sah per plugin — sama dengan `layouts()` di server. */
const LAYOUTS: Record<string, readonly string[]> = Object.fromEntries(PLUGINS.map((p) => [p.kind, (p.layouts ?? []).map((l) => l.value)]));
const STYLES = new Set(["plain", "card", "tint", "dark", "soft", "flat"]);
const SURFACES = new Set(["light", "cream", "dark"]);
const TEXT_SCALES = [0.9, 1.0, 1.1];
const NAVBAR_STYLES = new Set(["top", "side"]);
const BACKGROUND_SIZES = new Set(["cover", "contain", "repeat"]);
const HIDE_ON = new Set(["mobile", "desktop"]);

/** Batas kanvas: 12 kolom (ADR-0017) dan tinggi maksimum yang masih masuk akal untuk satu blok. */
export const COLS = 12;
export const MAX_BLOCK_H = 64;

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): string | undefined => (typeof v === "string" && v.trim() !== "" ? v : undefined);
const int = (v: unknown, min: number, max: number, fallback: number): number => {
  const n = typeof v === "number" ? v : typeof v === "string" ? Number(v) : NaN;
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n)));
};
const oneOf = (v: unknown, set: Set<string>): string | undefined => (typeof v === "string" && set.has(v) ? v : undefined);

function block(raw: unknown, index: number, taken: Set<string>): BlockV2 | null {
  if (!isObj(raw)) return null;
  const kind = oneOf(raw.kind, KINDS);
  if (!kind) return null; // fail-closed: plugin tak dikenal tidak dirender
  // `data` dibiarkan apa adanya: bentuk per plugin sudah dijaga validator server, dan renderer
  // membaca setiap field lewat helper `text()`/`safeColor()` yang tahan nilai tak terduga.
  const data = isObj(raw.data) ? (raw.data as BlockV2["data"]) : {};

  // Id unik: id tak sah diganti, id duplikat diberi sufiks (kunci React harus unik).
  let id = safeId(str(raw.id) ?? `blok-${index}`);
  if (taken.has(id)) {
    let n = 2;
    while (taken.has(`${id}-${n}`)) n += 1;
    id = `${id}-${n}`;
  }
  taken.add(id);

  const w = int(raw.w, 1, COLS, COLS);
  const h = int(raw.h, 1, MAX_BLOCK_H, 4);
  // x di-clamp agar blok tidak pernah keluar kanvas: x + w <= 12.
  const x = int(raw.x, 0, COLS - w, 0);
  const y = int(raw.y, 0, Number.MAX_SAFE_INTEGER, 0);

  const layout = str(raw.layout);
  const out: BlockV2 = {
    id,
    kind: kind as BlockV2["kind"],
    x,
    y,
    w,
    h,
    style: (oneOf(raw.style, STYLES) ?? "plain") as BlockV2["style"],
    data,
  };
  // `layout` hanya sah bila plugin memang punya varian itu (server menolak sebaliknya).
  if (layout && LAYOUTS[kind]?.includes(layout)) out.layout = layout;
  const hideOn = oneOf(raw.hide_on, HIDE_ON);
  if (hideOn) out.hide_on = hideOn as BlockV2["hide_on"];
  if (raw.hidden === true) out.hidden = true;
  if (typeof raw.m_order === "number" && Number.isInteger(raw.m_order) && raw.m_order >= 0 && raw.m_order <= 40) out.m_order = raw.m_order;
  if (raw.locked === true) out.locked = true;
  if (raw.auto_height === false) out.auto_height = false;
  if (typeof raw.chat_open === "boolean") out.chat_open = raw.chat_open;
  return out;
}

function seo(raw: unknown): PageV2["seo"] {
  if (!isObj(raw)) return undefined;
  const out: NonNullable<PageV2["seo"]> = {};
  for (const k of ["title", "description", "image", "favicon", "keywords"] as const) {
    const v = str(raw[k]);
    if (v) out[k] = v;
  }
  return Object.keys(out).length ? out : undefined;
}

/**
 * Dokumen v2 yang aman dirender. Bentuk apa pun diterima (`unknown`), termasuk `null`.
 * Data yang tidak sah dibuang atau di-clamp — tidak pernah melempar.
 */
export function normalizePageV2(raw: unknown): PageV2 {
  const m = isObj(raw) ? raw : {};
  const t = isObj(m.theme) ? m.theme : {};

  const theme: PageV2["theme"] = {
    // `primary`/`font` divalidasi lagi oleh `safeColor()`/`fontStack()` saat dirender, jadi nilai tak
    // sah cukup diteruskan; menyaringnya di sini akan menghilangkan pilihan pengguna yang sah.
    primary: str(t.primary) ?? "#2563EB",
    font: str(t.font) ?? "inter",
  };
  const surface = oneOf(t.surface, SURFACES);
  if (surface) theme.surface = surface as PageV2["theme"]["surface"];
  const scale = typeof t.text_scale === "number" ? t.text_scale : Number(t.text_scale);
  if (TEXT_SCALES.some((s) => Math.abs(s - scale) < 1e-9)) theme.text_scale = TEXT_SCALES.find((s) => Math.abs(s - scale) < 1e-9);
  const nav = oneOf(t.navbar_style, NAVBAR_STYLES);
  if (nav) theme.navbar_style = nav as PageV2["theme"]["navbar_style"];
  if (typeof t.navbar_sticky === "boolean") theme.navbar_sticky = t.navbar_sticky;
  if (t.hide_badge === true) theme.hide_badge = true;
  if (typeof t.heading_font === "string") theme.heading_font = t.heading_font;
  if (typeof t.heading_weight === "number" && [500, 600, 700, 800].includes(t.heading_weight)) theme.heading_weight = t.heading_weight;
  if (typeof t.accent === "string") theme.accent = t.accent;
  if (t.gradient === true) theme.gradient = true;
  const radius = oneOf(t.radius, new Set(["sharp", "soft", "round"]));
  if (radius) theme.radius = radius as PageV2["theme"]["radius"];
  const motion = oneOf(t.motion, new Set(["none", "subtle", "lively"]));
  if (motion) theme.motion = motion as PageV2["theme"]["motion"];
  const texture = oneOf(t.texture, new Set(["none", "grain", "paper", "dots"]));
  if (texture) theme.texture = texture as PageV2["theme"]["texture"];
  if (t.page_transition === true) theme.page_transition = true;
  const bg = str(t.background_image);
  if (bg) theme.background_image = bg;
  const bgSize = oneOf(t.background_size, BACKGROUND_SIZES);
  if (bgSize) theme.background_size = bgSize as PageV2["theme"]["background_size"];
  const bgPos = str(t.background_position);
  if (bgPos && bgPos.length <= 60) theme.background_position = bgPos;
  if (typeof t.background_opacity === "number" && Number.isFinite(t.background_opacity)) {
    theme.background_opacity = Math.min(100, Math.max(0, Math.round(t.background_opacity)));
  }

  const taken = new Set<string>();
  const blocks = Array.isArray(m.blocks)
    ? m.blocks.map((b, i) => block(b, i, taken)).filter((b): b is BlockV2 => b !== null)
    : [];

  const doc: PageV2 = {
    schema_version: 2,
    template: safeId(str(m.template) ?? "studio"),
    theme,
    blocks,
  };
  const s = seo(m.seo);
  if (s) doc.seo = s;
  if (m.article && typeof m.article === "object" && !Array.isArray(m.article)) doc.article = m.article as PageV2["article"];
  // `drawings` hanya relevan di editor; bentuknya sudah dijaga validator dan tidak memengaruhi tata letak.
  if (Array.isArray(m.drawings) && m.drawings.length > 0) doc.drawings = m.drawings as PageV2["drawings"];
  return doc;
}

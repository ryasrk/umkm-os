"use client";

// Pengaturan halaman aktif: SEO (judul, deskripsi, gambar bagikan) dan — untuk blog — metadata artikel
// (ringkasan, sampul, penulis, tanggal, tag) yang dipakai daftar artikel & JSON-LD Article di <head>.

import { ImageSquare, Trash } from "@phosphor-icons/react/ssr";

import { t as tx } from "@/i18n";
import type { ArticleMeta } from "@/renderer/article";
import type { StudioSeo } from "@/studio/doc";

function Thumb({ url }: { url?: string }) {
  // eslint-disable-next-line @next/next/no-img-element -- pratinjau aset bertanda tangan
  return url ? <img src={url} alt="" className="st-bg-thumb" /> : null;
}

const today = () => new Date().toISOString().slice(0, 10);

export function PageSettings({ seo, article, onSeo, onArticle, upload, assetUrl, isHome }: {
  seo: StudioSeo | undefined;
  article: ArticleMeta | undefined;
  onSeo: (next: StudioSeo) => void;
  onArticle: (next: ArticleMeta | undefined) => void;
  upload: (f: File) => Promise<string | null>;
  assetUrl: (ref: string) => string | undefined;
  isHome: boolean;
}) {
  const s = seo ?? {};
  const pick = async (f: File | undefined, done: (ref: string) => void) => { if (!f) return; const r = await upload(f); if (r) done(r); };
  return (
    <div className="st-section st-page-settings">
      <h3>{tx("Pengaturan halaman")}</h3>
      <label className="d-field">
        <span className="st-row-between"><span>{tx("Judul di Google")}</span><small className="d-muted">{(s.title ?? "").length}/70</small></span>
        <input maxLength={70} value={s.title ?? ""} placeholder={tx("Kosongkan untuk memakai nama halaman")} onChange={(e) => onSeo({ ...s, title: e.target.value })} />
      </label>
      <label className="d-field">
        <span className="st-row-between"><span>{tx("Deskripsi di Google")}</span><small className="d-muted">{(s.description ?? "").length}/160</small></span>
        <textarea id="seo-description" rows={3} maxLength={160} value={s.description ?? ""} placeholder={tx("1–2 kalimat yang membuat orang ingin mengklik.")} onChange={(e) => onSeo({ ...s, description: e.target.value })} />
      </label>
      <label className="d-field">
        <span className="st-row-between"><span>{tx("Kata kunci (SEO)")}</span><small className="d-muted">{(s.keywords ?? "").length}/200</small></span>
        <input maxLength={200} value={s.keywords ?? ""} placeholder={tx("mis. kopi susu, kedai kopi bandung")} onChange={(e) => onSeo({ ...s, keywords: e.target.value })} />
      </label>
      <div className="d-field">
        <span>{tx("Gambar saat dibagikan (WhatsApp, Facebook)")}</span>
        <div className="d-row">
          <Thumb url={s.image ? assetUrl(s.image) : undefined} />
          <label className="d-btn d-btn-sm"><ImageSquare weight="bold" aria-hidden="true" />{s.image ? tx("Ganti") : tx("Pilih gambar")}
            <input type="file" accept="image/png,image/jpeg,image/webp" hidden onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ""; void pick(f, (r) => onSeo({ ...s, image: r })); }} />
          </label>
          {s.image && <button type="button" className="d-btn d-btn-sm d-btn-ghost" onClick={() => onSeo({ ...s, image: undefined })}><Trash weight="bold" aria-hidden="true" />{tx("Hapus")}</button>}
        </div>
      </div>

      {!isHome && (
        <>
          <label className="d-switch u-mt-2">
            <input type="checkbox" checked={!!article} onChange={(e) => onArticle(e.target.checked ? { published_at: today() } : undefined)} />
            <span><strong>{tx("Halaman ini artikel/blog")}</strong><span className="d-muted">{tx("Muncul di Daftar artikel dan dikenali Google sebagai artikel (tanggal, penulis, sampul).")}</span></span>
          </label>
          {article && (
            <div className="st-article-fields">
              <label className="d-field"><span>{tx("Ringkasan")}</span>
                <textarea rows={3} maxLength={300} value={article.excerpt ?? ""} placeholder={tx("Tampil di kartu artikel & di bawah judul.")} onChange={(e) => onArticle({ ...article, excerpt: e.target.value })} />
              </label>
              <div className="d-field">
                <span>{tx("Gambar sampul")}</span>
                <div className="d-row">
                  <Thumb url={article.cover ? assetUrl(article.cover) : undefined} />
                  <label className="d-btn d-btn-sm"><ImageSquare weight="bold" aria-hidden="true" />{article.cover ? tx("Ganti") : tx("Pilih gambar")}
                    <input type="file" accept="image/png,image/jpeg,image/webp" hidden onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ""; void pick(f, (r) => onArticle({ ...article, cover: r })); }} />
                  </label>
                  {article.cover && <button type="button" className="d-btn d-btn-sm d-btn-ghost" onClick={() => onArticle({ ...article, cover: undefined })}><Trash weight="bold" aria-hidden="true" />{tx("Hapus")}</button>}
                </div>
              </div>
              <div className="d-grid">
                <label className="d-field u-m-0"><span>{tx("Penulis")}</span><input maxLength={80} value={article.author ?? ""} onChange={(e) => onArticle({ ...article, author: e.target.value })} /></label>
                <label className="d-field u-m-0"><span>{tx("Tanggal terbit")}</span><input type="date" value={article.published_at ?? ""} onChange={(e) => onArticle({ ...article, published_at: e.target.value || undefined })} /></label>
              </div>
              <label className="d-field u-mt-2"><span>{tx("Tag (pisahkan dengan koma, maks 8)")}</span>
                <input maxLength={260} value={(article.tags ?? []).join(", ")} placeholder={tx("mis. kopi, resep, tips")} onChange={(e) => onArticle({ ...article, tags: e.target.value.split(",").map((x) => x.trimStart()).slice(0, 8) })} />
              </label>
            </div>
          )}
        </>
      )}
    </div>
  );
}

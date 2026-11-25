import { t } from "@/i18n";
// Siapkan foto sebelum diunggah: API menolak body > ±2 MB (server.body_limit_bytes) dan memutus koneksi
// (browser hanya melihat ERR_CONNECTION_ABORTED). Foto HP biasanya 3–8 MB. Kompres gaya WhatsApp:
// sisi terpanjang ≤ 1600 px, WebP (fallback JPEG), target ±300 KB — kecil tapi tetap terlihat bagus
// di layar (tidak pernah di-zoom). PNG kecil transparan (stiker/logo) dikirim apa adanya.

export const MAX_UPLOAD_BYTES = 1_800_000;
const EXT: Record<string, string> = { "image/webp": "webp", "image/png": "png", "image/jpeg": "jpg" };

// Gaya WhatsApp: gambar dikompres AGRESIF — tetap terlihat bagus di layar (≤1600 px,
// target ±300 KB) karena tidak pernah di-zoom. Foto HP 3–8 MB jadi puluhan KB.
const TARGET_BYTES = 300_000;
const TARGET_SIDE = 1600;

export class ImagePrepError extends Error {}

function load(file: File): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => { URL.revokeObjectURL(url); resolve(img); };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new ImagePrepError(t("Format foto tidak bisa dibaca browser. Pakai JPEG, PNG, atau WebP."))); };
    img.src = url;
  });
}

const toBlob = (c: HTMLCanvasElement, type: string, q: number) =>
  new Promise<Blob | null>((resolve) => c.toBlob(resolve, type, q));

export async function prepareImage(file: File): Promise<File> {
  if (!file.type.startsWith("image/")) throw new ImagePrepError(t("Pilih file gambar (JPEG, PNG, atau WebP)."));
  // PNG kecil transparan (≤ 300 KB): aset vektor/stiker/ikon — dikirim apa adanya, tanpa direasterisasi.
  if (file.type === "image/png" && file.size <= TARGET_BYTES) return file;
  // Kompres gaya WhatsApp: SEMUA foto (walaupun sudah < 1,8 MB) dikecilkan ke ±300 KB.

  const img = await load(file);
  const ratio = Math.min(1, TARGET_SIDE / Math.max(img.naturalWidth, img.naturalHeight));
  let w = Math.max(1, Math.round(img.naturalWidth * ratio));
  let h = Math.max(1, Math.round(img.naturalHeight * ratio));
  const transparent = file.type === "image/png" || file.type === "image/webp";
  // PNG/WebP bisa transparan (stiker, logo, potongan objek): jangan pernah jadi JPEG yang meratakan
  // transparansi ke hitam/putih — WebP (alfa tetap) lalu PNG.
  const types = transparent ? ["image/webp", "image/png"] : ["image/webp", "image/jpeg"];
  const canvas = document.createElement("canvas");
  for (let attempt = 0; attempt < 6; attempt++) {
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) break;
    ctx.drawImage(img, 0, 0, w, h);
    for (const q of [0.8, 0.72, 0.64, 0.55, 0.45]) {
      for (const type of types) {
        // Browser yang tidak bisa meng-encode `type` mengembalikan PNG — hanya terima jenis yang diminta.
        const blob = await toBlob(canvas, type, q);
        if (blob && blob.type === type && blob.size <= TARGET_BYTES) {
          return new File([blob], file.name.replace(/\.[^.]+$/, "") + `.${EXT[type]}`, { type });
        }
      }
    }
    // Masih terlalu besar: perkecil dimensi lalu coba lagi.
    w = Math.round(w * 0.8);
    h = Math.round(h * 0.8);
  }
  // Fallback terakhir: kualitas terendah, dimensi minimum wajar (512 px) — asal lolos batas unggah.
  w = Math.min(512, Math.max(1, w));
  h = Math.max(1, Math.round(h * (w / Math.max(1, img.naturalWidth))));
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.drawImage(img, 0, 0, w, h);
    for (const type of transparent ? ["image/webp", "image/png"] : ["image/webp", "image/jpeg"]) {
      const blob = await toBlob(canvas, type, 0.5);
      if (blob && blob.type === type && blob.size <= MAX_UPLOAD_BYTES) {
        return new File([blob], file.name.replace(/\.[^.]+$/, "") + `.${EXT[type]}`, { type });
      }
    }
  }
  throw new ImagePrepError(t("Foto terlalu besar meski sudah dikompres. Coba foto lain."));
}

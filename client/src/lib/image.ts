/**
 * Compress an image File down to a small JPEG data URL before it goes into a
 * chat envelope. Harbor's relay only buffers JSON (no binary upload), so
 * attachments ride as base64 inside the existing `chat` payload — keeping them
 * small keeps the WS frame + relay outbox lightweight and respects the
 * private-relay, short-TTL model.
 *
 * Resize to fit within `maxSize` on the long edge (aspect preserved), encode
 * JPEG at `quality`. A phone photo (~3 MB) lands around 40-90 KB this way.
 * Returns a data URL string (suitable for <img src> / SQLite TEXT / JSON).
 */
export async function fileToCompressedDataUrl(
  file: File,
  maxSize = 900,
  quality = 0.72,
): Promise<string> {
  if (!file.type.startsWith("image/")) {
    throw new Error("Selecione um arquivo de imagem.");
  }
  // 10 MB hard cap on the input — prevents a giant RAW/GIF from stalling the
  // canvas decode before we ever get to compress it.
  if (file.size > 10 * 1024 * 1024) {
    throw new Error("Imagem muito grande (máx 10 MB).");
  }
  const dataUrl: string = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Falha ao ler o arquivo."));
    reader.onload = () => resolve(reader.result as string);
    reader.readAsDataURL(file);
  });
  const img = await loadImage(dataUrl);
  const { width, height } = fit(img.width, img.height, maxSize);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas indisponível.");
  ctx.drawImage(img, 0, 0, width, height);
  return canvas.toDataURL("image/jpeg", quality);
}

function fit(w: number, h: number, max: number): { width: number; height: number } {
  if (w <= max && h <= max) return { width: w, height: h };
  const scale = max / Math.max(w, h);
  return { width: Math.round(w * scale), height: Math.round(h * scale) };
}

/**
 * Compress an image File into a small SQUARE JPEG data URL for use as a profile
 * avatar. Center-crops to a square (cover, not contain) so the avatar is round
 * without letterboxing, then resizes to `size` and encodes JPEG at `quality`.
 * A phone photo lands around 8-20 KB this way — light enough to publish to the
 * relay (it stores the data-URL string and hands it to the partner verbatim).
 * Returns a data URL string (suitable for <img src>, identity.json, /profile).
 */
export async function fileToAvatarDataUrl(
  file: File,
  size = 256,
  quality = 0.7,
): Promise<string> {
  if (!file.type.startsWith("image/")) {
    throw new Error("Selecione um arquivo de imagem.");
  }
  if (file.size > 10 * 1024 * 1024) {
    throw new Error("Imagem muito grande (máx 10 MB).");
  }
  const dataUrl: string = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Falha ao ler o arquivo."));
    reader.onload = () => resolve(reader.result as string);
    reader.readAsDataURL(file);
  });
  const img = await loadImage(dataUrl);
  // Center-crop to a square: take the min of width/height as the crop edge,
  // centered, then draw onto a size×size canvas (cover, not contain).
  const edge = Math.min(img.width, img.height);
  const sx = (img.width - edge) / 2;
  const sy = (img.height - edge) / 2;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas indisponível.");
  ctx.drawImage(img, sx, sy, edge, edge, 0, 0, size, size);
  return canvas.toDataURL("image/jpeg", quality);
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("Imagem inválida ou corrompida."));
    img.src = src;
  });
}

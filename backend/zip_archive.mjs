import { inflateRawSync } from "node:zlib";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";

const EOCD = 0x06054b50;
const CENTRAL = 0x02014b50;
const LOCAL = 0x04034b50;
const MAX_FILES = 256;
const MAX_TOTAL_BYTES = 1024 * 1024 * 1024;

function endOfCentralDirectory(bytes) {
  const minimum = Math.max(0, bytes.length - 22 - 0xffff);
  for (let offset = bytes.length - 22; offset >= minimum; offset -= 1) {
    if (bytes.readUInt32LE(offset) === EOCD) return offset;
  }
  throw new Error("web update is not a supported ZIP archive");
}

function safeParts(name) {
  const normalized = String(name).replaceAll("\\", "/").replace(/^\.\//, "");
  if (!normalized || normalized.startsWith("/") || normalized.includes("\0")) return null;
  const parts = normalized.split("/").filter(Boolean);
  if (!parts.length || parts.some((part) => part === "." || part === "..")) return null;
  return parts;
}

function entries(bytes) {
  const end = endOfCentralDirectory(bytes);
  if (bytes.readUInt16LE(end + 4) !== 0 || bytes.readUInt16LE(end + 6) !== 0) {
    throw new Error("multi-disk ZIP archives are unsupported");
  }
  const count = bytes.readUInt16LE(end + 10);
  const centralOffset = bytes.readUInt32LE(end + 16);
  if (count > MAX_FILES || centralOffset >= bytes.length) throw new Error("web ZIP is too large");
  const result = [];
  let offset = centralOffset;
  let totalBytes = 0;
  for (let index = 0; index < count; index += 1) {
    if (offset + 46 > bytes.length || bytes.readUInt32LE(offset) !== CENTRAL) {
      throw new Error("web ZIP central directory is corrupt");
    }
    const flags = bytes.readUInt16LE(offset + 8);
    const method = bytes.readUInt16LE(offset + 10);
    const compressedSize = bytes.readUInt32LE(offset + 20);
    const uncompressedSize = bytes.readUInt32LE(offset + 24);
    const nameLength = bytes.readUInt16LE(offset + 28);
    const extraLength = bytes.readUInt16LE(offset + 30);
    const commentLength = bytes.readUInt16LE(offset + 32);
    const externalAttributes = bytes.readUInt32LE(offset + 38);
    const localOffset = bytes.readUInt32LE(offset + 42);
    const next = offset + 46 + nameLength + extraLength + commentLength;
    if (next > bytes.length || (flags & 1) || ![0, 8].includes(method)) {
      throw new Error("web ZIP contains an unsupported entry");
    }
    const name = bytes.subarray(offset + 46, offset + 46 + nameLength).toString("utf8");
    const parts = safeParts(name);
    const unixType = (externalAttributes >>> 16) & 0xf000;
    if (!parts || unixType === 0xa000) throw new Error("web ZIP contains an unsafe path");
    const directory = name.endsWith("/");
    if (!directory) {
      totalBytes += uncompressedSize;
      if (totalBytes > MAX_TOTAL_BYTES) throw new Error("web ZIP expands beyond the size limit");
    }
    result.push({ parts, directory, method, compressedSize, uncompressedSize, localOffset });
    offset = next;
  }
  return result;
}

function unpackEntry(bytes, entry) {
  const offset = entry.localOffset;
  if (offset + 30 > bytes.length || bytes.readUInt32LE(offset) !== LOCAL) {
    throw new Error("web ZIP local header is corrupt");
  }
  const nameLength = bytes.readUInt16LE(offset + 26);
  const extraLength = bytes.readUInt16LE(offset + 28);
  const start = offset + 30 + nameLength + extraLength;
  const end = start + entry.compressedSize;
  if (end > bytes.length) throw new Error("web ZIP entry is truncated");
  const compressed = bytes.subarray(start, end);
  const unpacked = entry.method === 0 ? Buffer.from(compressed) : inflateRawSync(compressed);
  if (unpacked.length !== entry.uncompressedSize) throw new Error("web ZIP entry size is invalid");
  return unpacked;
}

export async function publishZipDirectory(archivePath, destinationDirectory, {
  markerName = "",
  markerContents = "",
} = {}) {
  const bytes = await readFile(archivePath);
  const archiveEntries = entries(bytes);
  const files = archiveEntries.filter((entry) => !entry.directory);
  if (!files.length) throw new Error("web ZIP contains no files");
  const commonRoot = files.every((entry) => entry.parts.length > 1
    && entry.parts[0] === files[0].parts[0]) ? files[0].parts[0] : "";
  const destination = resolve(destinationDirectory);
  const staging = `${destination}.part-${process.pid}`;
  const previous = `${destination}.previous-${process.pid}`;
  await rm(staging, { recursive: true, force: true });
  await rm(previous, { recursive: true, force: true });
  await mkdir(staging, { recursive: true });
  try {
    for (const entry of files) {
      const relativeParts = commonRoot ? entry.parts.slice(1) : entry.parts;
      if (!relativeParts.length) continue;
      const output = join(staging, ...relativeParts);
      await mkdir(dirname(output), { recursive: true });
      await writeFile(output, unpackEntry(bytes, entry));
    }
    if (markerName) {
      const markerParts = safeParts(markerName);
      if (!markerParts || markerParts.length !== 1) {
        throw new Error("web publish marker name is unsafe");
      }
      await writeFile(join(staging, markerParts[0]), String(markerContents));
    }
    try {
      await rename(destination, previous);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    try {
      await rename(staging, destination);
    } catch (error) {
      try { await rename(previous, destination); } catch {}
      throw error;
    }
    await rm(previous, { recursive: true, force: true });
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
  return { files: files.length, directory: destination };
}

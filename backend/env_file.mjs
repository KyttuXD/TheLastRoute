import { readFile } from "node:fs/promises";

function decodeDoubleQuoted(value) {
  return value.replace(/\\([nrt\\"])/g, (_match, escaped) => {
    if (escaped === "n") return "\n";
    if (escaped === "r") return "\r";
    if (escaped === "t") return "\t";
    return escaped;
  });
}

export function parseEnvironmentFile(contents) {
  const parsed = new Map();
  const lines = String(contents).replace(/^\uFEFF/, "").split(/\r?\n/);

  for (const originalLine of lines) {
    let line = originalLine.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("export ")) line = line.slice(7).trimStart();

    const separator = line.indexOf("=");
    if (separator < 1) continue;
    const key = line.slice(0, separator).trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) continue;

    let value = line.slice(separator + 1).trim();
    if (value.startsWith('"')) {
      const closingQuote = value.lastIndexOf('"');
      if (closingQuote > 0) value = decodeDoubleQuoted(value.slice(1, closingQuote));
    } else if (value.startsWith("'")) {
      const closingQuote = value.lastIndexOf("'");
      if (closingQuote > 0) value = value.slice(1, closingQuote);
    } else {
      value = value.replace(/\s+#.*$/, "").trimEnd();
    }
    parsed.set(key, value);
  }

  return parsed;
}

export async function loadEnvironmentFile(path, environment = process.env) {
  let contents;
  try {
    contents = await readFile(path, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }

  for (const [key, value] of parseEnvironmentFile(contents)) {
    // Pterodactyl variables and variables supplied to `node` remain authoritative.
    if (!(key in environment)) environment[key] = value;
  }
  return true;
}

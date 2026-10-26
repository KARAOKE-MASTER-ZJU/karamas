import process from "node:process";
import { log, error } from "node:console";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";

const [version, ...extra] = process.argv.slice(2);
const number = "(?:0|[1-9][0-9]*)";
const prerelease = `(?:${number}|[0-9]*[A-Za-z-][0-9A-Za-z-]*)`;
const semver = new RegExp(
  `^${number}\\.${number}\\.${number}(?:-${prerelease}(?:\\.${prerelease})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`,
);

function replaceVersion(source, pattern, path) {
  const matches = [...source.matchAll(pattern)];
  if (matches.length !== 1) {
    throw new Error(`${path} 中未找到唯一的版本字段`);
  }
  return source.replace(
    pattern,
    (_, before, after) => `${before}${version}${after}`,
  );
}

try {
  if (!version || extra.length || !semver.test(version)) {
    throw new Error(
      "用法：pnpm version:set <版本号>，例如 0.2.0 或 0.2.0-beta.1",
    );
  }

  const paths = [
    "app/package.json",
    "app/src-tauri/Cargo.toml",
    "karamas_core/Cargo.toml",
    "Cargo.lock",
  ];
  const files = paths.map((path) => {
    const absolute = fileURLToPath(new URL(`../../${path}`, import.meta.url));
    return { path, absolute, original: readFileSync(absolute, "utf8") };
  });
  const app = JSON.parse(files[0].original);
  app.version = version;
  const updated = [
    `${JSON.stringify(app, null, 2)}\n`,
    ...files
      .slice(1, 3)
      .map((file) =>
        replaceVersion(
          file.original,
          /(^\[package\]\r?\n(?:(?!^\[)[\s\S])*?^version\s*=\s*")[^"]+("[^\r\n]*)/gm,
          file.path,
        ),
      ),
    ["karamas", "karamas_core"].reduce(
      (lock, name) =>
        replaceVersion(
          lock,
          new RegExp(
            `(^\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = ")[^"]+(")`,
            "gm",
          ),
          `${paths[3]} (${name})`,
        ),
      files[3].original,
    ),
  ];

  const written = [];
  try {
    for (const [index, file] of files.entries()) {
      written.push(file);
      writeFileSync(file.absolute, updated[index]);
    }
  } catch (cause) {
    for (const file of written) writeFileSync(file.absolute, file.original);
    throw cause;
  }
  log(`应用和核心库版本已统一更新为 ${version}`);
} catch (cause) {
  error(cause.message);
  process.exitCode = 1;
}

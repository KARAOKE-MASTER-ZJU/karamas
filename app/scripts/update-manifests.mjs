import { createHash } from "node:crypto";
import {
  createReadStream,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, join } from "node:path";
import process from "node:process";

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const writeJson = (path, data) =>
  writeFileSync(path, `${JSON.stringify(data, null, 2)}\n`);
const abis = ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"];
const [mode, ...args] = process.argv.slice(2);

if (mode === "android") {
  const [apk, abi, properties, output] = args;
  if (!abis.includes(abi)) throw new Error(`不支持的 Android ABI：${abi}`);
  const props = readFileSync(properties, "utf8");
  const version = props
    .match(/^tauri\.android\.versionName=(.+)$/m)?.[1]
    ?.trim();
  const versionCode = Number(
    props.match(/^tauri\.android\.versionCode=(\d+)/m)?.[1],
  );
  if (!version || !Number.isSafeInteger(versionCode) || versionCode <= 0)
    throw new Error("无法读取 APK 的构建版本");
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(apk)) hash.update(chunk);
  writeJson(output, {
    version,
    versionCode,
    abi,
    name: basename(apk),
    size: statSync(apk).size,
    sha256: hash.digest("hex"),
  });
} else if (mode === "release") {
  const [directory, releasePath, notesPath, version] = args;
  const release = readJson(releasePath);
  if (release.tag_name !== `v${version}` || !release.draft)
    throw new Error("更新清单只能写入对应版本的 Release 草稿");
  const notes = readFileSync(notesPath, "utf8").trim();
  const platforms = {};
  const archNames = {
    x86: "i686",
    i386: "i686",
    i686: "i686",
    x64: "x86_64",
    amd64: "x86_64",
    x86_64: "x86_64",
    arm64: "aarch64",
    aarch64: "aarch64",
  };
  const bundleNames = {
    exe: "nsis",
    "app.tar.gz": "app",
    AppImage: "appimage",
    deb: "deb",
  };
  const assetByName = new Map(
    release.assets.map((asset) => [asset.name, asset]),
  );
  for (const file of readdirSync(directory).filter((name) =>
    name.endsWith(".sig"),
  )) {
    const name = file.slice(0, -4);
    if (!name.startsWith(`karamas_${version}_`))
      throw new Error(`产物版本不匹配：${name}`);
    const match = name
      .slice(`karamas_${version}_`.length)
      .match(
        /^(windows|linux|darwin|macos)_(\w+?)(?:-setup)?\.(exe|app\.tar\.gz|AppImage|deb)$/,
      );
    if (!match) throw new Error(`无法识别桌面更新包：${name}`);
    const [, platform, arch, bundle] = match;
    if (!archNames[arch]) throw new Error(`无法识别架构：${arch}`);
    const key = `${platform === "macos" ? "darwin" : platform}-${archNames[arch]}-${bundleNames[bundle]}`;
    const asset = assetByName.get(name);
    const signature = readFileSync(join(directory, file), "utf8").trim();
    if (!asset || !signature || platforms[key])
      throw new Error(`更新包缺失、重复或签名为空：${name}`);
    platforms[key] = { url: asset.browser_download_url, signature };
  }
  const expected = [
    ...["i686", "x86_64", "aarch64"].map((arch) => `windows-${arch}-nsis`),
    ...["x86_64", "aarch64"].map((arch) => `darwin-${arch}-app`),
    ...["x86_64", "aarch64"].flatMap((arch) =>
      ["appimage", "deb"].map((bundle) => `linux-${arch}-${bundle}`),
    ),
  ];
  for (const target of expected)
    if (!platforms[target]) throw new Error(`缺少桌面更新包：${target}`);

  const androidPlatforms = {};
  let versionCode;
  for (const abi of abis) {
    const metadata = readJson(join(directory, `android-${abi}.json`));
    const asset = assetByName.get(metadata.name);
    if (
      metadata.version !== version ||
      metadata.abi !== abi ||
      !asset ||
      asset.size !== metadata.size ||
      !Number.isSafeInteger(metadata.versionCode) ||
      metadata.versionCode <= 0 ||
      !/^[a-f0-9]{64}$/.test(metadata.sha256) ||
      (versionCode !== undefined && versionCode !== metadata.versionCode)
    )
      throw new Error(`Android 更新包信息不一致：${abi}`);
    versionCode = metadata.versionCode;
    androidPlatforms[abi] = {
      url: asset.browser_download_url,
      size: metadata.size,
      sha256: metadata.sha256,
    };
  }
  writeJson(join(directory, "latest.json"), {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms,
  });
  writeJson(join(directory, "android-update.json"), {
    version,
    versionCode,
    notes,
    platforms: androidPlatforms,
  });
} else {
  throw new Error(
    "用法：update-manifests.mjs android <APK> <ABI> <tauri.properties> <输出文件>，或 release <目录> <Release JSON> <更新日志> <版本>",
  );
}

import { appendFileSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// The same version must be accepted by Windows MSI as well as Cargo/Tauri.
const releaseVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function tomlValue(section, key) {
  return section?.match(new RegExp(`^${key}\\s*=\\s*"([^"]+)"`, 'm'))?.[1];
}

export function checkReleaseVersion({ packageJson, tauriConfig, cargoToml, cargoLock }, tag) {
  const cargoPackage = cargoToml.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  const lockPackage = cargoLock.split(/^\[\[package\]\]\s*$/m)
    .find((section) => tomlValue(section, 'name') === 'agentbar');
  const versions = {
    'package.json': packageJson.version,
    'src-tauri/tauri.conf.json': tauriConfig.version,
    'src-tauri/Cargo.toml': tomlValue(cargoPackage, 'version'),
    'src-tauri/Cargo.lock (agentbar)': tomlValue(lockPackage, 'version'),
  };
  const version = packageJson.version;
  if (typeof version !== 'string' || !releaseVersion.test(version)) {
    throw new Error('版本号必须为 MAJOR.MINOR.PATCH 三段数字，以兼容所有平台安装包。');
  }
  const [major, minor, patch] = version.split('.').map(Number);
  if (major > 255 || minor > 255 || patch > 65535) {
    throw new Error('Windows MSI 要求 major/minor 不超过 255，patch 不超过 65535。');
  }
  for (const [file, actual] of Object.entries(versions)) {
    if (actual !== version) throw new Error(`${file} 的版本 ${actual ?? '(缺失)'} 与 ${version} 不一致。`);
  }
  if (tag !== undefined && tag !== `v${version}`) {
    throw new Error(`发布标签必须为 v${version}，实际为 ${tag || '(空)'}。`);
  }
  return { version, tag: `v${version}` };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const root = new URL('../', import.meta.url);
    const read = (path) => readFileSync(new URL(path, root), 'utf8');
    const result = checkReleaseVersion({
      packageJson: JSON.parse(read('package.json')),
      tauriConfig: JSON.parse(read('src-tauri/tauri.conf.json')),
      cargoToml: read('src-tauri/Cargo.toml'),
      cargoLock: read('src-tauri/Cargo.lock'),
    }, process.env.RELEASE_TAG);
    if (process.env.GITHUB_OUTPUT) {
      appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(result)
        .map(([key, value]) => `${key}=${value}\n`).join(''));
    }
    console.log(`版本检查通过：${result.tag}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

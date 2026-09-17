import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkReleaseVersion } from './check-release-version.mjs';

function files(version = '0.1.0') {
  return {
    packageJson: { version },
    tauriConfig: { version },
    cargoToml: `[package]\nname = "agentbar"\nversion = "${version}"\n\n[dependencies]\nversion = "9.9.9"\n`,
    cargoLock: `version = 4\n\n[[package]]\nname = "other"\nversion = "9.9.9"\n\n[[package]]\nname = "agentbar"\nversion = "${version}"\n\n[[package]]\nname = "another"\nversion = "8.8.8"\n`,
  };
}

test('检查四处应用版本，忽略其他依赖版本', () => {
  assert.deepEqual(checkReleaseVersion(files(), 'v0.1.0'), {
    version: '0.1.0', tag: 'v0.1.0',
  });
  assert.equal(checkReleaseVersion(files()).tag, 'v0.1.0');
});

test('版本范围同时兼容 Windows MSI', () => {
  assert.equal(checkReleaseVersion(files('255.255.65535')).version, '255.255.65535');
  for (const version of ['256.0.0', '0.256.0', '0.0.65536']) {
    assert.throws(() => checkReleaseVersion(files(version)), /Windows MSI/);
  }
});

test('拒绝标签与应用版本不一致或非版本标签', () => {
  for (const tag of ['v0.2.0', 'main', '0.1.0', '', 'v0.1.0\nextra=value']) {
    assert.throws(() => checkReleaseVersion(files(), tag), /发布标签必须/);
  }
});

test('拒绝任意一个文件的版本不一致或缺失', () => {
  for (const key of ['tauriConfig', 'cargoToml', 'cargoLock']) {
    const data = files();
    data[key] = key === 'tauriConfig' ? { version: '0.2.0' } : data[key].replace('0.1.0', '0.2.0');
    assert.throws(() => checkReleaseVersion(data), /不一致/);
  }
  const data = files();
  data.cargoLock = data.cargoLock.replace('name = "agentbar"', 'name = "missing"');
  assert.throws(() => checkReleaseVersion(data), /缺失/);
});

test('拒绝非法或安装包不采用的版本格式', () => {
  for (const version of ['01.2.3', '1.2', 'v1.2.3', '1.2.3+build.1', '1.2.3-beta.1', '1.2.3-01', '1.2.3-']) {
    assert.throws(() => checkReleaseVersion(files(version)), /版本号必须/);
  }
});

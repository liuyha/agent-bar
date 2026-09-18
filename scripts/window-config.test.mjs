import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

function readSettingsWindow(filename) {
  const config = JSON.parse(readFileSync(new URL(`../src-tauri/${filename}`, import.meta.url), 'utf8'));
  const settings = config.app.windows.find((window) => window.label === 'settings');
  assert.ok(settings, `${filename} 必须包含偏好设置窗口`);
  return settings;
}

test('macOS 配置覆盖后保持偏好设置窗口布局一致', () => {
  const base = readSettingsWindow('tauri.conf.json');
  const macOS = readSettingsWindow('tauri.macos.conf.json');

  for (const field of ['width', 'height', 'minWidth', 'resizable', 'url']) {
    assert.notEqual(base[field], undefined, `通用配置必须声明 ${field}`);
    assert.equal(macOS[field], base[field], `macOS 配置必须同步偏好设置的 ${field}`);
  }
  assert.ok(base.width >= 960, '偏好设置初始宽度至少为 960');
});

test('macOS 偏好设置保留透明背景与原生窗口效果', () => {
  const settings = readSettingsWindow('tauri.macos.conf.json');

  assert.equal(settings.transparent, true);
  assert.deepEqual(settings.windowEffects.effects, ['underWindowBackground']);
  assert.equal(settings.windowEffects.state, 'active');
});

test('macOS 偏好设置使用覆盖式标题栏并保留左侧原生窗口按钮', () => {
  const settings = readSettingsWindow('tauri.macos.conf.json');

  assert.equal(settings.titleBarStyle, 'Overlay');
  assert.equal(settings.hiddenTitle, true);
  assert.equal(settings.decorations, true);
  // Tao uses the close button's left edge for x; y enlarges its title-bar
  // container, retaining the native button's vertical origin. On this macOS
  // version, (13, 22) places the 14x16 button's center at (20, 24).
  assert.deepEqual(settings.trafficLightPosition, { x: 13, y: 22 });
});

test('自定义标题栏拖动与双击权限仅授权偏好设置窗口', () => {
  const capability = JSON.parse(readFileSync(new URL('../src-tauri/capabilities/settings-window.json', import.meta.url), 'utf8'));

  assert.deepEqual(capability.windows, ['settings']);
  assert.deepEqual(capability.permissions, [
    'core:window:allow-start-dragging',
    'core:window:allow-internal-toggle-maximize',
  ]);
});

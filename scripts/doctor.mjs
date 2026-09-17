import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { delimiter, join } from 'node:path';

const env = { ...process.env };
const pathKey = Object.keys(env).find((key) => key.toUpperCase() === 'PATH') ?? 'PATH';
env[pathKey] = [join(homedir(), '.cargo', 'bin'), env[pathKey]].filter(Boolean).join(delimiter);

const checks = [
  ['Node.js (>=24)', 'node', ['--version']],
  ['pnpm (>=11)', 'pnpm', ['--version']],
  ['Rust', 'rustc', ['--version']],
  ['Cargo', 'cargo', ['--version']],
];
if (process.platform === 'darwin') checks.push(['Xcode Command Line Tools', 'xcode-select', ['-p']]);
let failed = Number(process.versions.node.split('.')[0]) < 24;
for (const [label, command, args] of checks) {
  const result = spawnSync(command, args, { env, encoding: 'utf8', shell: process.platform === 'win32' });
  const ok = result.status === 0;
  if (!ok) failed = true;
  console.log(`${ok ? '✓' : '✗'} ${label}: ${ok ? result.stdout.trim() : '未找到，请按 README 安装并重新打开终端'}`);
}
if (failed) console.log('\n环境尚未就绪。仅预览界面可使用 pnpm dev；桌面端需要 Rust 和系统依赖。');
process.exitCode = failed ? 1 : 0;

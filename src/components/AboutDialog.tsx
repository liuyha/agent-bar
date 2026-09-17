import type { RefObject } from 'react';
import { Button } from './ui/button';
import { version } from '../../package.json';

export function AboutDialog({ dialogRef }: { dialogRef: RefObject<HTMLDialogElement | null> }) {
  return (
    <dialog ref={dialogRef} className="about-dialog" aria-labelledby="about-title" aria-describedby="about-description" onKeyDown={(event) => {
      if (event.key === 'Escape' && !event.nativeEvent.isComposing) {
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.close();
      }
    }}>
      <span className="brand-mark" aria-hidden="true"><i /><i /><i /></span>
      <h2 id="about-title">关于 AgentBar</h2>
      <p className="about-version">版本 {version}</p>
      <p id="about-description">AI 用量，一目了然。<br />在菜单栏查看 Codex 与 Claude 的账号用量和 Token 使用统计。</p>
      <form method="dialog"><Button type="submit" variant="secondary" size="sm">关闭</Button></form>
    </dialog>
  );
}

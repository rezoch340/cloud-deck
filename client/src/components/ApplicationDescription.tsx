import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Panel } from './ui.tsx';

export function ApplicationDescription({ text }: { text?: string }) {
  if (!text?.trim()) return null;
  return (
    <Panel title="应用说明">
      <div className="cd-markdown">
        <Markdown
          remarkPlugins={[remarkGfm]}
          skipHtml
          components={{
            table: ({ children }) => (
              <div className="cd-markdown-table">
                <table>{children}</table>
              </div>
            ),
            a: ({ children, href }) => (
              <a href={href} target="_blank" rel="noopener noreferrer">
                {children}
              </a>
            ),
            img: ({ alt }) => <span>{alt}</span>,
          }}
        >
          {text}
        </Markdown>
      </div>
    </Panel>
  );
}

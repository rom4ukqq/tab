import React from 'react';

export const agentNames: Record<string, string> = {
  codex: 'Codex', claude: 'Claude Code', omp: 'Oh My Pi', mock: 'Тестовый агент',
};

// Upstream SVG assets, see third_party/NOTICES.md for sources and licenses.
export function AgentIcon({ provider }: { provider: string }) {
  return <span className={`agent-icon agent-icon-${provider}`} aria-hidden="true">
    {provider === 'omp' ? <img src="/icons/omp.svg" alt=""/> : ['codex', 'claude'].includes(provider) ?
      <span className="agent-logo" style={{ maskImage: `url('/icons/${provider}.svg')`, WebkitMaskImage: `url('/icons/${provider}.svg')` }}/> : <span>?</span>}
  </span>;
}

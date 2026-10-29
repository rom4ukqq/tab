import React from 'react';

export const agentNames: Record<string, string> = {
  codex: 'Codex', claude: 'Claude Code', omp: 'Oh My Pi', mock: 'Тестовый агент',
};

// Local vector identifiers: distinct silhouettes remain readable without colour.
export function AgentIcon({ provider }: { provider: string }) {
  return <span className={`agent-icon agent-icon-${provider}`} aria-hidden="true">
    <svg viewBox="0 0 32 32" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {provider === 'codex' ? <>
        <rect x="3" y="5" width="26" height="22" rx="5"/>
        <path d="m9 11 5 5-5 5m9 0h5"/>
      </> : provider === 'claude' ? <>
        {Array.from({ length: 12 }, (_, i) => <path key={i} d={`M16 ${i % 2 ? 5 : 3}v8`} transform={`rotate(${i * 30} 16 16)`}/>)}
        <circle cx="16" cy="16" r="3" fill="currentColor" stroke="none"/>
      </> : provider === 'omp' ? <>
        <path d="M5 10c3-3 5-2 8-2h14M12 8c0 7 0 13-4 17m14-17v13c0 4 3 4 5 2" strokeWidth="3"/>
      </> : <>
        <rect x="5" y="9" width="22" height="18" rx="5"/>
        <path d="M16 4v5m-6 7h1m10 0h1m-10 6h8"/>
      </>}
    </svg>
  </span>;
}

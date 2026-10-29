import React from 'react';

export const backgrounds = [
  { id: 'matrix', name: 'Матрица' },
  { id: 'jellyfish', name: 'Медузы' },
  { id: 'orbit', name: 'Орбита' },
  { id: 'waves', name: 'Волны' },
  { id: 'none', name: 'Без фона' },
] as const;
export type Background = typeof backgrounds[number]['id'];
export const validBackground = (value: string | null): value is Background => backgrounds.some(b => b.id === value);
export const artworkStyle = (theme: Background): React.CSSProperties => ({
  maskImage: `url('/themes/${theme}.svg')`, WebkitMaskImage: `url('/themes/${theme}.svg')`,
});

export function BackgroundPicker({ value, onChange }: { value: Background; onChange: (value: Background) => void }) {
  return <fieldset className="background-picker"><legend>ASCII-фон</legend>
    <div className="background-options">{backgrounds.map(b => <button type="button" key={b.id}
      className="background-option" aria-pressed={value === b.id} onClick={() => onChange(b.id)}>
      <span className="background-preview" aria-hidden="true">
        {b.id === 'matrix' ? <span className="matrix-preview">01 : X 0<br/>: + 1 0 :<br/>X 0 : + 1</span> : b.id !== 'none' ? <span className="ascii-thumbnail" style={artworkStyle(b.id)}/> : <span className="plain-preview">—</span>}
      </span><span>{b.name}</span>
    </button>)}</div>
  </fieldset>;
}

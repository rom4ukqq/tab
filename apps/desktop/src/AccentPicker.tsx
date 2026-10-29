import React from 'react';

export const defaultAccent = '#FECA17';
export const validAccent = (value: string | null): value is string => !!value && /^#[0-9a-f]{6}$/i.test(value);
const choices = [
  ['Жёлтый', defaultAccent], ['Мятный', '#59D9B0'], ['Голубой', '#66B5FF'],
  ['Фиолетовый', '#B79AFF'], ['Розовый', '#F594BE'], ['Коралловый', '#F79875'],
];

export function accentColors(hex: string, dark: boolean) {
  const rgb = [1, 3, 5].map(offset => parseInt(hex.slice(offset, offset + 2), 16));
  const luminance = rgb.map(v => v / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4)
    .reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
  const mix = (target: number, amount: number) => '#' + rgb.map(v => Math.round(v + (target - v) * amount).toString(16).padStart(2, '0')).join('');
  return {
    hover: mix(255, .16), pressed: mix(0, .16),
    text: luminance > .179 ? '#000000' : '#FFFFFF',
    foreground: dark ? (luminance < .25 ? mix(255, .5) : hex) : (luminance > .12 ? mix(0, .6) : hex),
  };
}

export function AccentPicker({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  return <fieldset className="accent-picker">
    <legend>Акцентный цвет</legend>
    <div className="accent-options">
      {choices.map(([name, color]) => <button type="button" className="accent-swatch" key={color}
        style={{ backgroundColor: color }} aria-label={name} title={name}
        aria-pressed={value.toUpperCase() === color} onClick={() => onChange(color)}/>) }
      <label className="accent-custom">Свой цвет
        <span className="custom-color-chip" style={{ backgroundColor: value }}><input type="color" value={value} aria-label="Выбрать свой акцентный цвет" onChange={e => { if (validAccent(e.target.value)) onChange(e.target.value.toUpperCase()); }}/></span>
      </label>
      <output aria-label="Текущий цвет">{value.toUpperCase()}</output>
    </div>
  </fieldset>;
}

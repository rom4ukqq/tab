import React, { useEffect, useRef } from 'react';
const symbols = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ:;<>+*#%';
const noise = (n: number) => { const x = Math.sin(n * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };
export function MatrixBackdrop({ animate }: { animate: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null), elapsed = useRef(0);
  useEffect(() => {
    const canvas = ref.current!, ctx = canvas.getContext('2d')!;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)');
    let width = 0, height = 0, request = 0, previous = 0, ink = '#7dabb8', accent = '#f594be';
    const paint = () => {
      ctx.clearRect(0, 0, width, height); ctx.font = '13px Consolas, monospace'; ctx.textAlign = 'center';
      for (let col = 0; col < Math.ceil(width / 22); col++) {
        const length = 9 + Math.floor(noise(col + 2) * 18), cycle = height + length * 22 + 140;
        const head = (elapsed.current * (19 + noise(col + 8) * 27) + noise(col + 19) * cycle) % cycle;
        for (let row = 0; row < length; row++) {
          const y = head - row * 22; if (y < -22 || y > height + 22) continue;
          const phase = elapsed.current * .8 + noise(col * 71 + row) * 50, blend = Math.min(1, phase % 1 * 4);
          const char = (tick: number) => symbols[Math.floor(noise(col * 193 + row * 17 + tick * 29) * symbols.length)];
          const opacity = Math.pow(1 - row / length, 1.7) * .62;
          ctx.fillStyle = row === 0 || col % 5 === 0 ? accent : ink;
          ctx.globalAlpha = opacity * (1 - blend); ctx.fillText(char(Math.floor(phase) - 1), col * 22 + 11, y);
          ctx.globalAlpha = opacity * blend; ctx.fillText(char(Math.floor(phase)), col * 22 + 11, y);
        }
      }
      ctx.globalAlpha = 1;
    };
    const resize = () => { const rect = canvas.getBoundingClientRect(), dpr = Math.min(devicePixelRatio, 2); width = rect.width; height = rect.height; canvas.width = Math.round(width * dpr); canvas.height = Math.round(height * dpr); ctx.setTransform(dpr, 0, 0, dpr, 0, 0); paint(); };
    const colors = () => { const css = getComputedStyle(canvas); ink = css.getPropertyValue('--matrix-ink').trim(); accent = css.getPropertyValue('--hub-accent').trim(); paint(); };
    const loop = (now: number) => { if (previous) elapsed.current += Math.min((now - previous) / 1000, .1); previous = now; paint(); request = requestAnimationFrame(loop); };
    const resume = () => { cancelAnimationFrame(request); previous = 0; if (animate && !document.hidden && !reduce.matches) request = requestAnimationFrame(loop); else paint(); };
    const sizeObserver = new ResizeObserver(resize); sizeObserver.observe(canvas);
    const themeObserver = new MutationObserver(colors), shell = canvas.closest('.shell'); if (shell) themeObserver.observe(shell, { attributes: true, attributeFilter: ['class', 'style'] });
    colors(); resize(); resume(); document.addEventListener('visibilitychange', resume); reduce.addEventListener('change', resume);
    return () => { cancelAnimationFrame(request); sizeObserver.disconnect(); themeObserver.disconnect(); document.removeEventListener('visibilitychange', resume); reduce.removeEventListener('change', resume); };
  }, [animate]);
  return <div className="matrix-backdrop" aria-hidden="true"><canvas ref={ref} style={{ width: '100%', height: '100%', display: 'block' }}/></div>;
}

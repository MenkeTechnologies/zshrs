/**
 * stryke docs — theme / CRT / neon / color-scheme toggles.
 * Vendored and simplified from audio_haxor/docs/hud-theme.js.
 * Storage keys live under the `stryke-hud-*` namespace so the two projects
 * can't clobber each other if ever opened from the same origin.
 */
(function () {
  'use strict';

  var STORAGE = {
    theme: 'stryke-hud-theme',
    crt: 'stryke-hud-crt',
    neon: 'stryke-hud-neon',
    scheme: 'stryke-hud-scheme',
  };

  var SCHEME_VAR_KEYS = [
      '--accent', '--accent-light', '--accent-glow',
      '--cyan', '--cyan-glow', '--cyan-dim',
      '--magenta', '--magenta-glow',
      '--green', '--green-bg',
      '--yellow', '--yellow-glow',
      '--orange', '--orange-bg',
      '--red',
      '--text', '--text-dim', '--text-muted',
      '--bg-primary', '--bg-secondary', '--bg-card', '--bg-hover',
      '--border', '--border-glow',
  ];

  var SCHEME_ORDER = ['cyberpunk', 'midnight', 'matrix', 'ember', 'arctic', 'crimson', 'toxic', 'vapor'];

  var COLOR_SCHEMES = {
      cyberpunk: {
          label: 'Cyberpunk',
          desc: 'Hot pink + cyan neon (default)',
          vars: {
              '--accent': '#ff2a6d', '--accent-light': '#ff6b9d',
              '--accent-glow': 'rgba(255, 42, 109, 0.4)',
              '--cyan': '#05d9e8', '--cyan-glow': 'rgba(5, 217, 232, 0.4)',
              '--cyan-dim': 'rgba(5, 217, 232, 0.15)',
              '--magenta': '#d300c5', '--magenta-glow': 'rgba(211, 0, 197, 0.3)',
              '--green': '#39ff14', '--green-bg': 'rgba(57, 255, 20, 0.08)',
              '--yellow': '#f9f002', '--yellow-glow': 'rgba(249, 240, 2, 0.2)',
              '--orange': '#ff6b35', '--orange-bg': 'rgba(255, 107, 53, 0.1)',
              '--red': '#ff073a',
              '--text': '#e0f0ff', '--text-dim': '#7a8ba8', '--text-muted': '#3d4f6a',
              '--bg-primary': '#05050a', '--bg-secondary': '#0a0a14',
              '--bg-card': '#0d0d1a', '--bg-hover': '#12122a',
              '--border': '#1a1a3e', '--border-glow': '#2a1a4e',
          },
          lightVars: {
              '--accent': '#d6196e', '--accent-light': '#e84d8a',
              '--accent-glow': 'rgba(214, 25, 110, 0.15)',
              '--cyan': '#0891b2', '--cyan-glow': 'rgba(8, 145, 178, 0.2)',
              '--cyan-dim': 'rgba(8, 145, 178, 0.08)',
              '--magenta': '#a300a3', '--magenta-glow': 'rgba(163, 0, 163, 0.15)',
              '--green': '#15803d', '--green-bg': 'rgba(21, 128, 61, 0.08)',
              '--yellow': '#a16207', '--yellow-glow': 'rgba(161, 98, 7, 0.1)',
              '--orange': '#c2410c', '--orange-bg': 'rgba(194, 65, 12, 0.06)',
              '--red': '#dc2626',
              '--text': '#1e293b', '--text-dim': '#475569', '--text-muted': '#94a3b8',
              '--bg-primary': '#f0f2f5', '--bg-secondary': '#e4e7ec',
              '--bg-card': '#ffffff', '--bg-hover': '#f7f8fa',
              '--border': '#cbd5e1', '--border-glow': '#a5b4c8',
          }
      },
      midnight: {
          label: 'Midnight',
          desc: 'Deep blue + electric purple',
          vars: {
              '--accent': '#7c3aed', '--accent-light': '#a78bfa',
              '--accent-glow': 'rgba(124, 58, 237, 0.4)',
              '--cyan': '#38bdf8', '--cyan-glow': 'rgba(56, 189, 248, 0.4)',
              '--cyan-dim': 'rgba(56, 189, 248, 0.15)',
              '--magenta': '#6366f1', '--magenta-glow': 'rgba(99, 102, 241, 0.3)',
              '--green': '#34d399', '--green-bg': 'rgba(52, 211, 153, 0.08)',
              '--yellow': '#c084fc', '--yellow-glow': 'rgba(192, 132, 252, 0.2)',
              '--orange': '#818cf8', '--orange-bg': 'rgba(129, 140, 248, 0.1)',
              '--red': '#f472b6',
              '--text': '#e0e7ff', '--text-dim': '#94a3b8', '--text-muted': '#475569',
              '--bg-primary': '#050510', '--bg-secondary': '#0a0a1e',
              '--bg-card': '#0d0d28', '--bg-hover': '#141432',
              '--border': '#1e1e4a', '--border-glow': '#2e1e5a',
          },
          lightVars: {
              '--accent': '#6d28d9', '--accent-light': '#8b5cf6',
              '--accent-glow': 'rgba(109, 40, 217, 0.15)',
              '--cyan': '#0284c7', '--cyan-glow': 'rgba(2, 132, 199, 0.2)',
              '--cyan-dim': 'rgba(2, 132, 199, 0.08)',
              '--magenta': '#4f46e5', '--magenta-glow': 'rgba(79, 70, 229, 0.15)',
              '--green': '#059669', '--green-bg': 'rgba(5, 150, 105, 0.08)',
              '--yellow': '#7c3aed', '--yellow-glow': 'rgba(124, 58, 237, 0.1)',
              '--orange': '#6366f1', '--orange-bg': 'rgba(99, 102, 241, 0.06)',
              '--red': '#e11d48',
              '--text': '#1e1b4b', '--text-dim': '#4338ca', '--text-muted': '#a5b4fc',
              '--bg-primary': '#eef2ff', '--bg-secondary': '#e0e7ff',
              '--bg-card': '#ffffff', '--bg-hover': '#f5f3ff',
              '--border': '#c7d2fe', '--border-glow': '#a5b4fc',
          }
      },
      matrix: {
          label: 'Matrix',
          desc: 'Terminal green on black',
          vars: {
              '--accent': '#22c55e', '--accent-light': '#4ade80',
              '--accent-glow': 'rgba(34, 197, 94, 0.4)',
              '--cyan': '#39ff14', '--cyan-glow': 'rgba(57, 255, 20, 0.4)',
              '--cyan-dim': 'rgba(57, 255, 20, 0.15)',
              '--magenta': '#16a34a', '--magenta-glow': 'rgba(22, 163, 74, 0.3)',
              '--green': '#4ade80', '--green-bg': 'rgba(74, 222, 128, 0.08)',
              '--yellow': '#a3e635', '--yellow-glow': 'rgba(163, 230, 53, 0.2)',
              '--orange': '#86efac', '--orange-bg': 'rgba(134, 239, 172, 0.1)',
              '--red': '#ef4444',
              '--text': '#d1fae5', '--text-dim': '#6ee7b7', '--text-muted': '#365314',
              '--bg-primary': '#020a02', '--bg-secondary': '#061006',
              '--bg-card': '#081408', '--bg-hover': '#0e200e',
              '--border': '#1a3a1a', '--border-glow': '#1a4a1a',
          },
          lightVars: {
              '--accent': '#16a34a', '--accent-light': '#22c55e',
              '--accent-glow': 'rgba(22, 163, 74, 0.15)',
              '--cyan': '#15803d', '--cyan-glow': 'rgba(21, 128, 61, 0.2)',
              '--cyan-dim': 'rgba(21, 128, 61, 0.08)',
              '--magenta': '#166534', '--magenta-glow': 'rgba(22, 101, 52, 0.15)',
              '--green': '#22c55e', '--green-bg': 'rgba(34, 197, 94, 0.08)',
              '--yellow': '#65a30d', '--yellow-glow': 'rgba(101, 163, 13, 0.1)',
              '--orange': '#4ade80', '--orange-bg': 'rgba(74, 222, 128, 0.06)',
              '--red': '#dc2626',
              '--text': '#14532d', '--text-dim': '#166534', '--text-muted': '#86efac',
              '--bg-primary': '#f0fdf4', '--bg-secondary': '#dcfce7',
              '--bg-card': '#ffffff', '--bg-hover': '#f0fdf4',
              '--border': '#bbf7d0', '--border-glow': '#86efac',
          }
      },
      ember: {
          label: 'Ember',
          desc: 'Warm amber + orange tones',
          vars: {
              '--accent': '#f59e0b', '--accent-light': '#fbbf24',
              '--accent-glow': 'rgba(245, 158, 11, 0.4)',
              '--cyan': '#fb923c', '--cyan-glow': 'rgba(251, 146, 60, 0.4)',
              '--cyan-dim': 'rgba(251, 146, 60, 0.15)',
              '--magenta': '#ea580c', '--magenta-glow': 'rgba(234, 88, 12, 0.3)',
              '--green': '#84cc16', '--green-bg': 'rgba(132, 204, 22, 0.08)',
              '--yellow': '#fde047', '--yellow-glow': 'rgba(253, 224, 71, 0.2)',
              '--orange': '#f97316', '--orange-bg': 'rgba(249, 115, 22, 0.1)',
              '--red': '#dc2626',
              '--text': '#fef3c7', '--text-dim': '#d97706', '--text-muted': '#92400e',
              '--bg-primary': '#0a0502', '--bg-secondary': '#120a04',
              '--bg-card': '#1a0e06', '--bg-hover': '#24140a',
              '--border': '#3e2a1a', '--border-glow': '#4e3a1a',
          },
          lightVars: {
              '--accent': '#d97706', '--accent-light': '#f59e0b',
              '--accent-glow': 'rgba(217, 119, 6, 0.15)',
              '--cyan': '#ea580c', '--cyan-glow': 'rgba(234, 88, 12, 0.2)',
              '--cyan-dim': 'rgba(234, 88, 12, 0.08)',
              '--magenta': '#c2410c', '--magenta-glow': 'rgba(194, 65, 12, 0.15)',
              '--green': '#65a30d', '--green-bg': 'rgba(101, 163, 13, 0.08)',
              '--yellow': '#a16207', '--yellow-glow': 'rgba(161, 98, 7, 0.1)',
              '--orange': '#c2410c', '--orange-bg': 'rgba(194, 65, 12, 0.06)',
              '--red': '#dc2626',
              '--text': '#451a03', '--text-dim': '#92400e', '--text-muted': '#fbbf24',
              '--bg-primary': '#fffbeb', '--bg-secondary': '#fef3c7',
              '--bg-card': '#ffffff', '--bg-hover': '#fffbeb',
              '--border': '#fde68a', '--border-glow': '#fbbf24',
          }
      },
      arctic: {
          label: 'Arctic',
          desc: 'Cool whites + icy blue',
          vars: {
              '--accent': '#0ea5e9', '--accent-light': '#38bdf8',
              '--accent-glow': 'rgba(14, 165, 233, 0.4)',
              '--cyan': '#67e8f9', '--cyan-glow': 'rgba(103, 232, 249, 0.4)',
              '--cyan-dim': 'rgba(103, 232, 249, 0.15)',
              '--magenta': '#06b6d4', '--magenta-glow': 'rgba(6, 182, 212, 0.3)',
              '--green': '#2dd4bf', '--green-bg': 'rgba(45, 212, 191, 0.08)',
              '--yellow': '#a5f3fc', '--yellow-glow': 'rgba(165, 243, 252, 0.2)',
              '--orange': '#22d3ee', '--orange-bg': 'rgba(34, 211, 238, 0.1)',
              '--red': '#f43f5e',
              '--text': '#ecfeff', '--text-dim': '#a5f3fc', '--text-muted': '#155e75',
              '--bg-primary': '#020a0e', '--bg-secondary': '#041218',
              '--bg-card': '#061a22', '--bg-hover': '#0a2430',
              '--border': '#1a3a4e', '--border-glow': '#1a4a5e',
          },
          lightVars: {
              '--accent': '#0284c7', '--accent-light': '#0ea5e9',
              '--accent-glow': 'rgba(2, 132, 199, 0.15)',
              '--cyan': '#0891b2', '--cyan-glow': 'rgba(8, 145, 178, 0.2)',
              '--cyan-dim': 'rgba(8, 145, 178, 0.08)',
              '--magenta': '#0e7490', '--magenta-glow': 'rgba(14, 116, 144, 0.15)',
              '--green': '#0d9488', '--green-bg': 'rgba(13, 148, 136, 0.08)',
              '--yellow': '#155e75', '--yellow-glow': 'rgba(21, 94, 117, 0.1)',
              '--orange': '#06b6d4', '--orange-bg': 'rgba(6, 182, 212, 0.06)',
              '--red': '#e11d48',
              '--text': '#164e63', '--text-dim': '#0e7490', '--text-muted': '#a5f3fc',
              '--bg-primary': '#ecfeff', '--bg-secondary': '#cffafe',
              '--bg-card': '#ffffff', '--bg-hover': '#ecfeff',
              '--border': '#a5f3fc', '--border-glow': '#67e8f9',
          }
      },
      crimson: {
          label: 'Crimson',
          desc: 'Rose-red accent + teal highlight',
          vars: {
              '--accent': '#e11d48', '--accent-light': '#fb7185',
              '--accent-glow': 'rgba(225, 29, 72, 0.4)',
              '--cyan': '#2dd4bf', '--cyan-glow': 'rgba(45, 212, 191, 0.4)',
              '--cyan-dim': 'rgba(45, 212, 191, 0.15)',
              '--magenta': '#f43f5e', '--magenta-glow': 'rgba(244, 63, 94, 0.3)',
              '--green': '#22c55e', '--green-bg': 'rgba(34, 197, 94, 0.08)',
              '--yellow': '#fbbf24', '--yellow-glow': 'rgba(251, 191, 36, 0.2)',
              '--orange': '#fb923c', '--orange-bg': 'rgba(251, 146, 60, 0.1)',
              '--red': '#ff073a',
              '--text': '#ffe4e6', '--text-dim': '#b08a92', '--text-muted': '#6b4a52',
              '--bg-primary': '#0a0506', '--bg-secondary': '#140a0c',
              '--bg-card': '#1a0d10', '--bg-hover': '#2a1318',
              '--border': '#3e1a22', '--border-glow': '#4e2030',
          },
          lightVars: {
              '--accent': '#be123c', '--accent-light': '#e11d48',
              '--accent-glow': 'rgba(190, 18, 57, 0.15)',
              '--cyan': '#0d9488', '--cyan-glow': 'rgba(13, 148, 136, 0.2)',
              '--cyan-dim': 'rgba(13, 148, 136, 0.08)',
              '--magenta': '#be185d', '--magenta-glow': 'rgba(190, 24, 93, 0.15)',
              '--green': '#15803d', '--green-bg': 'rgba(21, 128, 61, 0.08)',
              '--yellow': '#a16207', '--yellow-glow': 'rgba(161, 98, 7, 0.1)',
              '--orange': '#c2410c', '--orange-bg': 'rgba(194, 65, 12, 0.06)',
              '--red': '#dc2626',
              '--text': '#1e293b', '--text-dim': '#475569', '--text-muted': '#94a3b8',
              '--bg-primary': '#faf0f1', '--bg-secondary': '#f2e4e6',
              '--bg-card': '#ffffff', '--bg-hover': '#fdf6f7',
              '--border': '#e0c5cb', '--border-glow': '#c8a5ad',
          }
      },
      toxic: {
          label: 'Toxic',
          desc: 'Acid-lime accent + magenta',
          vars: {
              '--accent': '#c6ff00', '--accent-light': '#e2ff6b',
              '--accent-glow': 'rgba(198, 255, 0, 0.4)',
              '--cyan': '#00e5ff', '--cyan-glow': 'rgba(0, 229, 255, 0.4)',
              '--cyan-dim': 'rgba(0, 229, 255, 0.15)',
              '--magenta': '#ff00aa', '--magenta-glow': 'rgba(255, 0, 170, 0.3)',
              '--green': '#39ff14', '--green-bg': 'rgba(57, 255, 20, 0.08)',
              '--yellow': '#f9f002', '--yellow-glow': 'rgba(249, 240, 2, 0.2)',
              '--orange': '#ff6b35', '--orange-bg': 'rgba(255, 107, 53, 0.1)',
              '--red': '#ff073a',
              '--text': '#e8ffd0', '--text-dim': '#8a9a6a', '--text-muted': '#4a5a32',
              '--bg-primary': '#07090a', '--bg-secondary': '#0c0f0a',
              '--bg-card': '#0f130c', '--bg-hover': '#161b10',
              '--border': '#2a3a1a', '--border-glow': '#3a4a20',
          },
          lightVars: {
              '--accent': '#5c8a00', '--accent-light': '#7cab1a',
              '--accent-glow': 'rgba(92, 138, 0, 0.15)',
              '--cyan': '#0891b2', '--cyan-glow': 'rgba(8, 145, 178, 0.2)',
              '--cyan-dim': 'rgba(8, 145, 178, 0.08)',
              '--magenta': '#a3006e', '--magenta-glow': 'rgba(163, 0, 110, 0.15)',
              '--green': '#15803d', '--green-bg': 'rgba(21, 128, 61, 0.08)',
              '--yellow': '#a16207', '--yellow-glow': 'rgba(161, 98, 7, 0.1)',
              '--orange': '#c2410c', '--orange-bg': 'rgba(194, 65, 12, 0.06)',
              '--red': '#dc2626',
              '--text': '#1e293b', '--text-dim': '#475569', '--text-muted': '#94a3b8',
              '--bg-primary': '#f3f7ec', '--bg-secondary': '#e7eedb',
              '--bg-card': '#ffffff', '--bg-hover': '#f8fbf2',
              '--border': '#d0dcc0', '--border-glow': '#b0c098',
          }
      },
      vapor: {
          label: 'Vapor',
          desc: 'Vaporwave pastel pink + cyan',
          vars: {
              '--accent': '#ff6ec7', '--accent-light': '#ff9fd8',
              '--accent-glow': 'rgba(255, 110, 199, 0.4)',
              '--cyan': '#72f1ff', '--cyan-glow': 'rgba(114, 241, 255, 0.4)',
              '--cyan-dim': 'rgba(114, 241, 255, 0.15)',
              '--magenta': '#c792ea', '--magenta-glow': 'rgba(199, 146, 234, 0.3)',
              '--green': '#5af2b0', '--green-bg': 'rgba(90, 242, 176, 0.08)',
              '--yellow': '#fff59d', '--yellow-glow': 'rgba(255, 245, 157, 0.2)',
              '--orange': '#ffb38a', '--orange-bg': 'rgba(255, 179, 138, 0.1)',
              '--red': '#ff6b8b',
              '--text': '#f0e6ff', '--text-dim': '#a99cc4', '--text-muted': '#6a5f86',
              '--bg-primary': '#0d0814', '--bg-secondary': '#140d1f',
              '--bg-card': '#1a1228', '--bg-hover': '#241836',
              '--border': '#2e2142', '--border-glow': '#3e2d56',
          },
          lightVars: {
              '--accent': '#d6469e', '--accent-light': '#ff6ec7',
              '--accent-glow': 'rgba(214, 70, 158, 0.15)',
              '--cyan': '#0e9bb0', '--cyan-glow': 'rgba(14, 155, 176, 0.2)',
              '--cyan-dim': 'rgba(14, 155, 176, 0.08)',
              '--magenta': '#8b5cf6', '--magenta-glow': 'rgba(139, 92, 246, 0.15)',
              '--green': '#10a37f', '--green-bg': 'rgba(16, 163, 127, 0.08)',
              '--yellow': '#b59000', '--yellow-glow': 'rgba(181, 144, 0, 0.1)',
              '--orange': '#d97757', '--orange-bg': 'rgba(217, 119, 87, 0.06)',
              '--red': '#e11d48',
              '--text': '#2a1e3a', '--text-dim': '#5a4a72', '--text-muted': '#9a8ab0',
              '--bg-primary': '#f7f0fb', '--bg-secondary': '#ede2f5',
              '--bg-card': '#ffffff', '--bg-hover': '#faf4fd',
              '--border': '#ddc9ec', '--border-glow': '#c4a8db',
          }
      },
  };
    // ===== end canonical data =====

  function readStored(key) {
    try { return localStorage.getItem(key); } catch (_) { return null; }
  }
  function writeStored(key, val) {
    try { localStorage.setItem(key, val); } catch (_) {}
  }

  function currentScheme() {
    return readStored(STORAGE.scheme) || 'cyberpunk';
  }

  function applyScheme(name) {
    var scheme = COLOR_SCHEMES[name];
    if (!scheme) return;
    // Inline `style.setProperty` on :root beats the CSS variable in
    // `hud-static.css` (`:root { --accent: …; }`). Without choosing
    // between `vars` (dark) and `lightVars` (light) by current theme,
    // toggling `data-theme="light"` does nothing — the inline dark
    // values stay pinned.
    var theme = document.documentElement.getAttribute('data-theme') || 'dark';
    var vars = (theme === 'light' && scheme.lightVars) ? scheme.lightVars : (scheme.vars || {});
    var root = document.documentElement;
    SCHEME_VAR_KEYS.forEach(function (k) {
      if (vars[k]) root.style.setProperty(k, vars[k]);
      else root.style.removeProperty(k);
    });
    writeStored(STORAGE.scheme, name);
    renderSchemeGrid(name);
  }

  function renderSchemeGrid(activeName) {
    var grid = document.getElementById('hudSchemeGrid');
    if (!grid) return;
    grid.innerHTML = '';
    SCHEME_ORDER.forEach(function (name) {
      var s = COLOR_SCHEMES[name];
      if (!s) return;
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'scheme-btn' + (name === activeName ? ' active' : '');
      btn.setAttribute('data-scheme', name);
      var n = document.createElement('div');
      n.className = 'scheme-btn-name';
      n.textContent = s.label;
      var d = document.createElement('div');
      d.className = 'scheme-btn-desc';
      d.textContent = s.desc;
      var prev = document.createElement('div');
      prev.className = 'scheme-btn-preview';
      ['--accent', '--cyan', '--magenta', '--green'].forEach(function (k) {
        var dot = document.createElement('span');
        dot.className = 'scheme-dot';
        dot.style.background = s.vars[k];
        prev.appendChild(dot);
      });
      btn.appendChild(n);
      btn.appendChild(d);
      btn.appendChild(prev);
      btn.addEventListener('click', function () { applyScheme(name); });
      grid.appendChild(btn);
    });
  }

  function applyTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    writeStored(STORAGE.theme, theme);
    var btn = document.getElementById('btnTheme');
    if (btn) btn.textContent = theme === 'light' ? 'Dark' : 'Light';
    // Re-apply the scheme so its light/dark variant takes effect.
    applyScheme(currentScheme());
  }

  function applyCrt(on) {
    var app = document.querySelector('.app');
    if (!app) return;
    app.classList.toggle('no-crt', !on);
    var h = document.getElementById('crtH');
    var v = document.getElementById('crtV');
    if (h) h.hidden = !on;
    if (v) v.hidden = !on;
    writeStored(STORAGE.crt, on ? '1' : '0');
    var btn = document.getElementById('btnCrt');
    if (btn) btn.classList.toggle('active', on);
  }

  function applyNeon(on) {
    document.body.classList.toggle('no-neon-glow', !on);
    writeStored(STORAGE.neon, on ? '1' : '0');
    var btn = document.getElementById('btnNeon');
    if (btn) btn.classList.toggle('active', on);
  }

  document.addEventListener('DOMContentLoaded', function () {
    var theme = readStored(STORAGE.theme) || 'dark';
    applyTheme(theme);

    var crt = readStored(STORAGE.crt);
    applyCrt(crt === null ? true : crt === '1');

    var neon = readStored(STORAGE.neon);
    applyNeon(neon === null ? true : neon === '1');
    // auto-inject color-scheme strip if the page lacks the markup (shared-chrome model)
    if (!document.getElementById('hudSchemeGrid')) {
      var _strip = document.createElement('div');
      _strip.className = 'hub-scheme-strip';
      _strip.innerHTML = '<div class="hub-scheme-strip-inner"><span class="hud-scheme-label">// Color scheme</span><div class="scheme-grid" id="hudSchemeGrid"></div></div>';
      var _hdr = document.querySelector('header');
      if (_hdr && _hdr.parentNode) _hdr.parentNode.insertBefore(_strip, _hdr.nextSibling);
      else document.body.insertBefore(_strip, document.body.firstChild);
    }


    var scheme = readStored(STORAGE.scheme) || 'cyberpunk';
    applyScheme(scheme);

    var btnTheme = document.getElementById('btnTheme');
    if (btnTheme) btnTheme.addEventListener('click', function () {
      var cur = document.documentElement.getAttribute('data-theme');
      applyTheme(cur === 'light' ? 'dark' : 'light');
    });

    var btnCrt = document.getElementById('btnCrt');
    if (btnCrt) btnCrt.addEventListener('click', function () {
      applyCrt(btnCrt.classList.contains('active') ? false : true);
    });

    var btnNeon = document.getElementById('btnNeon');
    if (btnNeon) btnNeon.addEventListener('click', function () {
      applyNeon(btnNeon.classList.contains('active') ? false : true);
    });
  });
})();

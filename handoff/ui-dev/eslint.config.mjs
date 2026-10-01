const browser = Object.fromEntries(['window', 'document', 'location', 'history', 'sessionStorage', 'fetch', 'Headers', 'URL',
  'CustomEvent', 'EventTarget', 'requestAnimationFrame', 'setTimeout', 'clearTimeout', 'getComputedStyle', 'matchMedia',
  'ResizeObserver', 'performance', 'AbortController', 'console', 'Float64Array', 'navigator', 'Blob', 'ClipboardItem',
  'localStorage', 'BroadcastChannel', 'URLSearchParams', 'DOMException'].map((name) => [name, 'readonly']))
export default [{
  files: ['**/*.js'],
  languageOptions: { ecmaVersion: 2024, sourceType: 'module', globals: browser },
  rules: { 'no-unused-vars': 'error', 'no-undef': 'error', 'no-unreachable': 'error', 'no-dupe-keys': 'error',
    'no-self-assign': 'error', 'no-constant-condition': 'error', 'eqeqeq': 'error', 'prefer-const': 'error', 'no-var': 'error' }
}]

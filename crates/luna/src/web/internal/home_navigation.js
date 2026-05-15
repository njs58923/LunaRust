// luna://internal/home_navigation.js
// Bind de botones de la home a sus rutas. Auto-incluido por LUNA_HOME.

const btnDemos = hiperspace.dimention.getElementById('btn_demos');
const btnSettings = hiperspace.dimention.getElementById('btn_settings');
const btnAbout = hiperspace.dimention.getElementById('btn_about');

if (btnDemos) btnDemos.addEventListener('toque', () => { location.href = 'luna://demos'; });
if (btnSettings) btnSettings.addEventListener('toque', () => { location.href = 'luna://settings'; });
if (btnAbout) btnAbout.addEventListener('toque', () => { location.href = 'luna://about'; });

console.log('[luna://home] Navigation ready');

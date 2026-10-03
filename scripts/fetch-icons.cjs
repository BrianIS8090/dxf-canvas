// Закреплённые исходники Lucide; сеть и sharp нужны только при обновлении иконок.
const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require('sharp');
const commit = 'aace268b9be318c4f6d8a09a35139f860d07d9c5';
const names = ['folder-open', 'scan', 'layout-grid', 'sliders-horizontal', 'circle-question-mark',
  'sun', 'moon', 'mouse-pointer-2', 'ruler', 'diameter', 'radius', 'triangle', 'pentagon',
  'undo-2', 'list', 'scan-line', 'panel-right', 'files', 'layers', 'search', 'x',
  'focus', 'minus', 'plus', 'download', 'trash'];
(async () => {
  const dir = path.join(__dirname, '../assets/lucide');
  await fs.mkdir(dir, { recursive: true });
  for (const name of [...names, 'LICENSE']) {
    const remote = name === 'LICENSE' ? 'LICENSE' : `icons/${name}.svg`;
    const response = await fetch(`https://raw.githubusercontent.com/lucide-icons/lucide/${commit}/${remote}`);
    if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
    const svg = await response.text();
    await fs.writeFile(path.join(dir, name === 'LICENSE' ? 'LICENSE.txt' : `${name}.svg`), svg);
    if (name !== 'LICENSE') {
      await sharp(Buffer.from(svg.replace('currentColor', '#ffffff')), { density: 288 })
        .resize(96, 96).png().toFile(path.join(dir, `${name}.png`));
    }
  }
  await fs.writeFile(path.join(dir, 'source.json'), JSON.stringify({ repository: 'https://github.com/lucide-icons/lucide', commit, names, size: 96 }, null, 2) + '\n');
  console.log(`Подготовлено ${names.length} иконок Lucide`);
})();



import { promises as fs } from "fs";
import path from "path";

const args = process.argv.slice(2);

if (args.length < 2) {
console.log(`
Uso:
node juntar-codigo.mjs <archivo_salida.txt> <ruta1> <ruta2> [...]

Ejemplo:
node juntar-codigo.mjs codigo-completo.txt src package.json index.mjs
`);
process.exit(1);
}

const outputFile = path.resolve(args[0]);
const inputPaths = args.slice(1).map(p => path.resolve(p));

const IGNORE_DIRS = new Set([
"node_modules",
".git",
"dist",
"build",
".next",
"coverage",
"target"
]);

const IGNORE_EXTENSIONS = new Set([
".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".txt",
".mp3", ".wav", ".ogg", ".mp4", ".mov",
".zip", ".rar", ".7z", ".gz",
".pdf", ".exe", ".dll", ".so", ".class",
".ttf", ".otf", ".woff", ".woff2",
".lock"
]);

async function exists(p) {
try {
  await fs.access(p);
  return true;
} catch {
  return false;
}
}

async function isDirectory(p) {
try {
  const stats = await fs.stat(p);
  return stats.isDirectory();
} catch {
  return false;
}
}

async function collectFiles(targetPath) {
const stats = await fs.stat(targetPath);

if (stats.isFile()) {
  return [targetPath];
}

if (stats.isDirectory()) {
  const entries = await fs.readdir(targetPath, { withFileTypes: true });
  let files = [];

  for (const entry of entries) {
    if (IGNORE_DIRS.has(entry.name)) continue;

    const fullPath = path.join(targetPath, entry.name);

    if (entry.isDirectory()) {
      files.push(...await collectFiles(fullPath));
    } else if (entry.isFile()) {
      if (IGNORE_EXTENSIONS.has(path.extname(entry.name).toLowerCase())) {
        continue;
      }
      files.push(fullPath);
    }
  }

  return files;
}

return [];
}

function isProbablyText(buffer) {
for (let i = 0; i < buffer.length; i++) {
  if (buffer[i] === 0) return false;
}
return true;
}

async function readAsText(filePath) {
const buffer = await fs.readFile(filePath);
if (!isProbablyText(buffer)) return null;
return buffer.toString("utf8");
}

async function main() {
if (await isDirectory(outputFile)) {
  console.error(`Error: la salida debe ser un archivo, no una carpeta:\n${outputFile}`);
  console.error(`Ejemplo correcto:\nnode juntar-codigo.mjs "salida.txt" "ruta1" "ruta2"`);
  process.exit(1);
}

let allFiles = [];

for (const inputPath of inputPaths) {
  if (!(await exists(inputPath))) {
    console.warn(`Ruta no encontrada: ${inputPath}`);
    continue;
  }

  const files = await collectFiles(inputPath);
  allFiles.push(...files);
}

allFiles = [...new Set(allFiles)].sort((a, b) => a.localeCompare(b));

let output = "";

for (const file of allFiles) {
  try {
    const content = await readAsText(file);
    if (content === null) continue;

    const relativePath = path.relative(process.cwd(), file) || file;

    output += "########################\n";
    output += `# ${relativePath}\n`;
    output += `${content}\n\n`;
  } catch (err) {
    const relativePath = path.relative(process.cwd(), file) || file;
    output += "########################\n";
    output += `# ${relativePath}\n`;
    output += `[ERROR LEYENDO ARCHIVO: ${err.message}]\n\n`;
  }
}

await fs.writeFile(outputFile, output, "utf8");

console.log(`Archivo generado: ${outputFile}`);
console.log(`Archivos incluidos: ${allFiles.length}`);
}

main().catch(err => {
console.error("Error:", err);
process.exit(1);
});
import { readdir, readFile, writeFile, rm } from 'node:fs/promises';
import { join, extname } from 'node:path';
import { brotliCompressSync, gzipSync, constants } from 'node:zlib';

const root = process.argv[2];
if (!root) throw new Error('Usage: node scripts/precompress.mjs <static-output-directory>');
const extensions = new Set(['.html', '.css', '.js', '.json', '.svg', '.txt', '.xml', '.wasm']);
let files = 0;
async function compressDirectory(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await compressDirectory(path);
    else if (entry.isFile() && extensions.has(extname(entry.name))) {
      const source = await readFile(path);
      for (const [suffix, data] of [
        ['.br', brotliCompressSync(source, { params: { [constants.BROTLI_PARAM_QUALITY]: 9 } })],
        ['.gz', gzipSync(source, { level: 9 })],
      ]) {
        if (data.length < source.length) await writeFile(path + suffix, data);
        else await rm(path + suffix, { force: true });
      }
      files++;
    }
  }
}
await compressDirectory(root);
console.log(`Prepared Brotli/gzip variants for ${files} static files.`);

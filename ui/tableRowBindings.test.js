import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const activeTableViews = [
  './views/CloudView.vue',
  './components/shares/ReceiveShareDialog.vue',
];

test('active Antdv Next tables bind row interactions through on-row', async () => {
  const sources = await Promise.all(
    activeTableViews.map((path) => readFile(new URL(path, import.meta.url), 'utf8')),
  );

  for (const source of sources) {
    assert.doesNotMatch(source, /:custom-row=/);
  }

  assert.equal(sources[0].match(/:on-row="fileRowProps"/g)?.length, 1);
  assert.equal(sources[0].match(/:on-row="folderPickerRowProps"/g)?.length, 1);
  assert.equal(sources[1].match(/:on-row="rowProps"/g)?.length, 1);
});

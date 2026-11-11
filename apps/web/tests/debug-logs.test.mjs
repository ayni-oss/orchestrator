import { test } from 'node:test';
import assert from 'node:assert/strict';
import { displayLog, appendLogPage, decodeLogEntries } from '../src/lib/debug-logs.ts';
test('raw rendering leaves markup as inert text and exposes terminal controls', () => {
  const bytes = new Uint8Array([0,255,27,91,50,74,13,10]);
  const result = displayLog(Buffer.from(bytes).toString('base64'));
  assert.equal(result, '\\u0000�\\u001b[2J\\u000d\n');
  assert.equal(displayLog(Buffer.from('<script>alert(1)</script>').toString('base64')), '<script>alert(1)</script>');
});
test('cursor identity preserves repeated identical output while bounding rendering', () => {
  const entries = Array.from({length: 1100}, (_,i) => ({cursor:String(i),record:{data:'c2FtZQo='}}));
  const retained = appendLogPage([], {entries});
  assert.equal(retained.length,1000);
  assert.equal(retained[0].cursor,'100');
  assert.equal(appendLogPage(retained,{entries:[entries[1099]]}).length,1000);
});

test('UTF-8 split across chunks is decoded in source order', () => {
  const record={source:{id:'a'},stream:'stdout'};
  const entries=[{cursor:'1',record:{...record,data:Buffer.from([0xe2,0x82]).toString('base64')}},{cursor:'2',record:{...record,data:Buffer.from([0xac]).toString('base64')}}];
  assert.equal([...decodeLogEntries(entries).values()].join(''),'€');
});

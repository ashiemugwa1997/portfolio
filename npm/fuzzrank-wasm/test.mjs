import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createIndex, highlight } from './index.js'

const lines = ['Deployment pipeline with automatic rollback', 'SAML single sign-on', 'Budget approvals', 'Café système']

test('search ranks and needs every word', async () => {
  const idx = await createIndex(lines)
  assert.equal(idx.size, 4)
  assert.deepEqual(idx.search('dep roll'), [0])
  assert.deepEqual(idx.search('saml budget'), [])
  assert.deepEqual(idx.search('CAFÉ'), [3])
  assert.deepEqual(idx.search('   '), [])
  assert.equal(idx.search('e', 2).length, 2)
})

test('positions and highlight', async () => {
  const idx = await createIndex(lines)
  assert.deepEqual(idx.positions(0, 'dply'), [0, 2, 3, 5])
  assert.deepEqual(idx.positions(1, 'dply'), [])
  assert.equal(highlight('a<b>', [1, 2]), 'a<mark>&#60;b</mark>&#62;')
  assert.equal(highlight('Deployment', idx.positions(0, 'dep')), '<mark>Dep</mark>loyment')
})

test('newlines inside a line do not shift indexes', async () => {
  const idx = await createIndex(['one\ntwo', 'three'])
  assert.equal(idx.size, 2)
  assert.deepEqual(idx.search('three'), [1])
})

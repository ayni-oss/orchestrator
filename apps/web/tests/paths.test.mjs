import test from 'node:test';
import assert from 'node:assert/strict';
import { workspacePath, workspaceSettingsPath, projectSettingsPath, repositoryPath, withCursor } from '../src/lib/paths.ts';

test('resource paths encode each ID and never turn resource identity into query or path structure', () => {
 assert.equal(projectSettingsPath('a/b', 'x?workspace=other'), '/workspaces/a%2Fb/projects/x%3Fworkspace%3Dother/settings');
 assert.equal(workspaceSettingsPath('a#b'), '/workspaces/a%23b/settings');
 assert.equal(repositoryPath('a/b'), '/repositories/a%2Fb');
 assert.throws(() => workspacePath(''));
});
test('pagination leaves workspace scope in the path and encodes opaque cursors', () => {
 const path = workspacePath('scope');
 assert.equal(withCursor(path, 'afterProject', 'a&b=c'), '/workspaces/scope?afterProject=a%26b%3Dc');
 assert.equal(withCursor(path, 'afterProject', ''), path);
});

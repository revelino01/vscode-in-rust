'use strict';

const assert = require('assert');
const path = require('path');

const vscode = require(path.resolve(__dirname, '../../../ext-host-js/vscode-api/index'));

console.log('[Phase 9 JS E2E] SignatureHelp & MarkdownString...');
const md = new vscode.MarkdownString('**Sum** values');
md.appendCodeblock('fn sum(a: i32, b: i32) -> i32', 'rust');
assert(md.value.includes('```rust'));

const pA = new vscode.ParameterInformation('a: i32', 'First operand');
const pB = new vscode.ParameterInformation('b: i32', 'Second operand');
const sigInfo = new vscode.SignatureInformation('sum(a: i32, b: i32) -> i32', md);
sigInfo.parameters = [pA, pB];
sigInfo.activeParameter = 1;
const sigHelp = new vscode.SignatureHelp();
sigHelp.signatures = [sigInfo];
sigHelp.activeSignature = 0;
sigHelp.activeParameter = 1;
assert.strictEqual(sigHelp.signatures[0].parameters.length, 2);

console.log('[Phase 9 JS E2E] SemanticTokensBuilder delta encoding...');
const legend = new vscode.SemanticTokensLegend(['function', 'variable'], ['declaration', 'readonly']);
const builder = new vscode.SemanticTokensBuilder(legend);
builder.push(0, 3, 4, 0, 1); // line 0, col 3, len 4, function, declaration
builder.push(0, 10, 1, 1, 2); // same line deltaChar = 7
builder.push(2, 4, 5, 1, 0); // deltaLine = 2, col 4
const tokens = builder.build('v1');
assert.strictEqual(tokens.resultId, 'v1');
assert.deepStrictEqual(Array.from(tokens.data), [0, 3, 4, 0, 1, 0, 7, 1, 1, 2, 2, 4, 5, 1, 0]);

console.log('[Phase 9 JS E2E] InlayHint, CodeLens, DocumentLink, Hierarchy...');
const hint = new vscode.InlayHint(new vscode.Position(0, 12), [new vscode.InlayHintLabelPart(': i32')], vscode.InlayHintKind.Type);
assert.strictEqual(hint.kind, vscode.InlayHintKind.Type);

const lens = new vscode.CodeLens(new vscode.Range(0, 0, 0, 10), { title: 'Run Test', command: 'test.run' });
assert.strictEqual(lens.command.command, 'test.run');

const linkUri = vscode.Uri.file('/workspace/README.md');
const docLink = new vscode.DocumentLink(new vscode.Range(1, 0, 1, 10), linkUri);
assert(docLink.target.includes('README.md'));

const itemUri = vscode.Uri.file('/workspace/src/main.rs');
const r = new vscode.Range(0, 0, 0, 8);
const callItem = new vscode.CallHierarchyItem(vscode.SymbolKind.Function, 'main', 'crate::main', itemUri, r, r);
const incoming = new vscode.CallHierarchyIncomingCall(callItem, [r]);
const outgoing = new vscode.CallHierarchyOutgoingCall(callItem, [r]);
const typeItem = new vscode.TypeHierarchyItem(vscode.SymbolKind.Struct, 'Engine', 'crate::Engine', itemUri, r, r);
assert.strictEqual(incoming.from.name, 'main');
assert.strictEqual(outgoing.to.name, 'main');
assert.strictEqual(typeItem.name, 'Engine');

console.log('[Phase 9 JS E2E] SnippetString & WorkspaceEdit...');
const snip = new vscode.SnippetString().appendText('fn ').appendTabstop(1).appendText('() {}');
assert.strictEqual(snip.value, 'fn $1() {}');

const wsEdit = new vscode.WorkspaceEdit();
wsEdit.replace(itemUri, new vscode.Range(0, 0, 0, 2), 'pub fn');
assert.strictEqual(wsEdit.size, 1);

console.log('[Phase 9 JS E2E] languages.register*Provider registrations...');
const disposables = [
  vscode.languages.registerSignatureHelpProvider({ language: 'rust' }, { provideSignatureHelp: () => sigHelp }, '(', ','),
  vscode.languages.registerImplementationProvider('rust', { provideImplementation: () => [new vscode.Location(itemUri, r)] }),
  vscode.languages.registerTypeDefinitionProvider('rust', { provideTypeDefinition: () => [new vscode.Location(itemUri, r)] }),
  vscode.languages.registerRenameProvider('rust', { provideRenameEdits: () => wsEdit }),
  vscode.languages.registerDocumentSemanticTokensProvider('rust', { provideDocumentSemanticTokens: () => tokens }, legend),
  vscode.languages.registerInlayHintsProvider('rust', { provideInlayHints: () => [hint] }),
  vscode.languages.registerCodeLensProvider('rust', { provideCodeLenses: () => [lens] }),
  vscode.languages.registerDocumentLinkProvider('rust', { provideDocumentLinks: () => [docLink] }),
  vscode.languages.registerCallHierarchyProvider('rust', {
    prepareCallHierarchy: () => [callItem],
    provideCallHierarchyIncomingCalls: () => [incoming],
    provideCallHierarchyOutgoingCalls: () => [outgoing],
  }),
  vscode.languages.registerTypeHierarchyProvider('rust', {
    prepareTypeHierarchy: () => [typeItem],
    provideTypeHierarchySupertypes: () => [typeItem],
    provideTypeHierarchySubtypes: () => [],
  }),
];
assert.strictEqual(disposables.length, 10);
for (const d of disposables) {
  assert.strictEqual(typeof d.dispose, 'function');
  d.dispose();
}
console.log('[Phase 9 JS E2E] PASSED');

const vscode = require('vscode');

class OutlineNode {
  constructor(label, children = [], explicitId = undefined, resourceUri = undefined) {
    this.label = label;
    this.children = children;
    this.id = explicitId;
    this.resourceUri = resourceUri;
  }
}

function activate(context) {
  const onDidChangeEmitter = new vscode.EventEmitter();
  let refreshCount = 0;

  const rootNodes = [
    new OutlineNode('src', [
      new OutlineNode('main.rs', [], undefined, vscode.Uri.file('/workspace/src/main.rs')),
      new OutlineNode('lib.rs', [], 'explicit_lib_rs', vscode.Uri.file('/workspace/src/lib.rs')),
    ]),
    new OutlineNode('Cargo.toml', [], 'explicit_cargo_toml'),
  ];

  const provider = {
    onDidChangeTreeData: onDidChangeEmitter.event,
    getTreeItem(element) {
      const state = element.children && element.children.length > 0
        ? vscode.TreeItemCollapsibleState.Collapsed
        : vscode.TreeItemCollapsibleState.None;
      const item = new vscode.TreeItem(element.label, state);
      if (element.id) {
        item.id = element.id;
      }
      if (element.resourceUri) {
        item.resourceUri = element.resourceUri;
      }
      item.tooltip = `Outline: ${element.label} (refresh=${refreshCount})`;
      item.contextValue = element.children && element.children.length > 0 ? 'folder' : 'file';
      return item;
    },
    getChildren(element) {
      if (!element) {
        return rootNodes;
      }
      return element.children || [];
    },
  };

  const treeView = vscode.window.createTreeView('projectOutlineView', {
    treeDataProvider: provider,
  });

  const refreshCmd = vscode.commands.registerCommand('projectOutline.refresh', (targetNode) => {
    refreshCount += 1;
    onDidChangeEmitter.fire(targetNode);
    return { refreshed: true, refreshCount };
  });

  if (context && Array.isArray(context.subscriptions)) {
    context.subscriptions.push(treeView, refreshCmd, onDidChangeEmitter);
  }

  return {
    getRefreshCount: () => refreshCount,
  };
}

function deactivate() {}

module.exports = {
  activate,
  deactivate,
};

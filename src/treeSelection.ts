export const within = (path: string, parent: string) => parent === "/" || path === parent || path.startsWith(parent + "/");

/** Keep whole-folder selections compact; split ancestors when a child is unchecked. */
export function toggleSelection(selected: Set<string>, path: string, children: ReadonlyMap<string, readonly { path: string }[]>): Set<string> {
  const next = new Set(selected);
  const ancestor = [...next].find(parent => within(path, parent));
  if (ancestor !== undefined) {
    next.delete(ancestor);
    let parent = ancestor;
    while (parent !== path) {
      const entries = children.get(parent);
      if (!entries) throw new Error("Expand this folder before changing its child selection.");
      const branch = entries.find(entry => within(path, entry.path));
      if (!branch) throw new Error("The selected path is no longer in this folder. Refresh the tree.");
      for (const entry of entries) if (entry !== branch) next.add(entry.path);
      parent = branch.path;
    }
  } else {
    for (const item of next) if (within(item, path)) next.delete(item);
    next.add(path);
  }
  return next;
}

// CodeMirror 6 的對外介面：打包成 IIFE，掛在 window.CM 上供 crates/editor/web/app.js 使用。
export { EditorState, Compartment, EditorSelection, Transaction } from '@codemirror/state';
export {
  EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection,
  dropCursor, highlightSpecialChars, rectangularSelection, crosshairCursor, placeholder,
} from '@codemirror/view';
export {
  defaultKeymap, history, historyKeymap, indentWithTab, undo, redo, undoDepth, redoDepth, selectAll,
} from '@codemirror/commands';
export { markdown, markdownLanguage, insertNewlineContinueMarkup, deleteMarkupBackward } from '@codemirror/lang-markdown';
export { syntaxHighlighting, HighlightStyle, indentUnit, bracketMatching, foldGutter, foldKeymap } from '@codemirror/language';
export { search, searchKeymap, highlightSelectionMatches, openSearchPanel } from '@codemirror/search';
export { tags } from '@lezer/highlight';

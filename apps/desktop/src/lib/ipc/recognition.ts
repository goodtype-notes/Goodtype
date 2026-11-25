import { invoke } from "@tauri-apps/api/core";
import type { ImporterStatus } from "./importer";
import { mathRecognitionAddon, type RecognitionAddon } from "./addons";

export type InkPolylines = { widthPt: number; points: [number, number][] }[];
export type RecognitionCapabilities = {
  recognizers: string[];
  math: ImporterStatus;
  /** The catalog entry for math, loaded with the capabilities so the panel can show it at once. */
  mathAddon: RecognitionAddon | null;
};
export type MathRecognition = { typst: string | null; unsupported: string | null; latex: string };

export async function recognitionCapabilities(): Promise<RecognitionCapabilities> {
  const [found, mathAddon] = await Promise.all([
    invoke<Omit<RecognitionCapabilities, "mathAddon">>("recognition_capabilities"),
    mathRecognitionAddon().catch(() => undefined),
  ]);
  return { ...found, mathAddon: mathAddon ?? null };
}
export const removeMathRecognizer = () => invoke<void>("remove_math_recognizer");
export const prepareMathRecognition = () => invoke<void>("prepare_math_recognition");
export function recognizeInkText(strokes: InkPolylines, recognizer: string | null): Promise<{ text: string }> {
  return invoke("recognize_ink_text", { selection: { strokes: strokes.map(s => s.points) }, recognizer });
}
/** The notebook and page bound a warm recognizer; neither is sent to the addon. */
export type MathContext = { root: string; page: { widthPt: number; heightPt: number } };
export function recognizeInkMath(strokes: InkPolylines, context: MathContext): Promise<MathRecognition> {
  return invoke("recognize_ink_math", { selection: { strokes }, context });
}

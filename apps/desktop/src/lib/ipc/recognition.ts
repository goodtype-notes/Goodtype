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
export function recognizeInkMath(strokes: InkPolylines): Promise<MathRecognition> {
  return invoke("recognize_ink_math", { selection: { strokes } });
}

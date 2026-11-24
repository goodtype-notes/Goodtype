import { contributionText, type AddonContribution, type AddonField } from "../ipc/addons";
import type { ShapeStyle } from "../model";
import type { AppSettings } from "../settings";
import type { PaletteDock } from "./palette";
import { spacingForWidth } from "./widths";

export type AddonStrokeSlot = {
  style: ShapeStyle;
  widths: number[];
  colors: string[];
};

export function defaultAddonStrokeSlot(settings: AppSettings, penIndex: number): AddonStrokeSlot {
  const pen = settings.penPresets[penIndex];
  return {
    style: {
      strokeColor: pen.color,
      strokeWidthPt: pen.widthPt,
      fillColor: null,
      opacity: 1,
      pattern: pen.pattern,
      patternSpacing: spacingForWidth(settings.penWidthSpacings, pen.widthPt),
    },
    widths: [...settings.penWidths],
    colors: [...settings.penSwatches],
  };
}

export function fitsAddonSegments(dock: PaletteDock, available: number, base: number, segments: number) {
  return (dock === "top" || dock === "bottom") && available >= 930 && base + segments + 30 <= available;
}

export function toolFieldPresentation(addon: AddonContribution, field: AddonField, locale: string) {
  if (field.type !== "choice") return field.type;
  if (field.choices.length <= 5 && field.choices.every(choice => choice.icon)) return "icons";
  if (field.choices.length <= 4 && field.choices.every(choice =>
    Array.from(contributionText(addon, choice.label, locale)).length <= 12)) return "text-segments";
  return "list";
}

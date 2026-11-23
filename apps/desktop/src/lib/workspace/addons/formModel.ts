import { contributionText, type AddonContribution, type AddonField, type AddonValue, type AddonValues } from "../../ipc/addons";

export function accepts(field: AddonField, value: AddonValue | undefined): boolean {
  switch (field.type) {
    case "toggle": return typeof value === "boolean";
    case "choice": return field.choices.some(choice => choice.value === value);
    case "text":
      if (typeof value !== "string" || [...value].length > field.maxLength || /\p{Cc}/u.test(value)) return false;
      if (field.pattern === "identifier") return value.length <= 48 && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value);
      if (field.pattern === "number") return /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(value) && Number.isFinite(Number(value));
      return true;
    case "number": {
      if (typeof value !== "number" || !Number.isFinite(value) || value < field.min || value > field.max) return false;
      const ticks = (value - field.min) / field.step;
      return Math.abs(ticks - Math.round(ticks)) <= 1e-8;
    }
  }
}

export const formDefaults = (fields: AddonField[]): AddonValues =>
  Object.fromEntries(fields.map(field => [field.id, field.default]));

export function matchesField(addon: AddonContribution, field: AddonField, query: string, locale: string): boolean {
  const keys = ["name", field.label, field.hint, field.search];
  const words = keys.map(key => key ? contributionText(addon, key, locale) : "").join(" ").toLocaleLowerCase(locale);
  return query.trim().toLocaleLowerCase(locale).split(/\s+/).every(word => words.includes(word));
}

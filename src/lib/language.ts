export function isChineseLanguage(language?: string | null) {
  const value = language?.trim().toLowerCase().replaceAll("_", "-") || "";
  return value.startsWith("zh") || value === "chinese" || value === "中文" || value === "cmn" || value === "zho";
}

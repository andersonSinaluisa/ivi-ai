// Prepara una respuesta escrita para leerla en voz alta.

export function toSpeech(markdown: string): string {
  return markdown
    .replace(/```[\s\S]*?```/g, " (bloque de código omitido) ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\((https?:[^)]+)\)/g, "$1")
    .replace(/https?:\/\/\S+/g, "el enlace")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/(\*\*|__|\*|_)(.*?)\1/g, "$2")
    .replace(/\s+/g, " ")
    .trim();
}

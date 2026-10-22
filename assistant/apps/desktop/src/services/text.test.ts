import { describe, expect, it } from "vitest";
import { toSpeech } from "./text";

describe("toSpeech", () => {
  it("quita markdown, código y URLs", () => {
    expect(toSpeech("## Listo\n- **Abrí** el `notepad`\nVer https://ollama.com")).toBe("Listo Abrí el notepad Ver el enlace");
    expect(toSpeech("Código:\n```js\nx()\n```")).toBe("Código: (bloque de código omitido)");
  });
});

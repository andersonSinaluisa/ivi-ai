import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMockBackend, setBackend } from "../../services/backend";
import { initialAssistantState, useAssistant } from "../../stores/assistantStore";
import { ChatScreen } from "./ChatScreen";

afterEach(() => {
  cleanup();
  useAssistant.setState(initialAssistantState);
});

describe("ChatScreen", () => {
  it("envía un mensaje y muestra la respuesta en streaming", async () => {
    setBackend(createMockBackend(1));
    render(<ChatScreen />);
    fireEvent.change(screen.getByLabelText("Mensaje"), { target: { value: "hola" } });
    fireEvent.click(screen.getByRole("button", { name: "Enviar" }));
    expect(await screen.findByText("hola")).toBeTruthy();
    await waitFor(() => expect(screen.getByText(/Modo vista previa/)).toBeTruthy());
  });

  it("pide autorización y ejecuta al aprobar", async () => {
    setBackend(createMockBackend(1));
    render(<ChatScreen />);
    await act(async () => {
      await useAssistant.getState().send("cierra el bloc de notas");
    });
    const approve = await screen.findByRole("button", { name: "Autorizar" });
    fireEvent.click(approve);
    await waitFor(() => expect(screen.getByText("Listo, cerré el Bloc de notas.")).toBeTruthy());
    expect(screen.getByText(/Hecho/)).toBeTruthy();
  });
});

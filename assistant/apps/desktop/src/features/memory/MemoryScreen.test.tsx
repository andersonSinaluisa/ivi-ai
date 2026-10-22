import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMockBackend, setBackend } from "../../services/backend";
import { MemoryScreen } from "./MemoryScreen";

afterEach(cleanup);

describe("MemoryScreen", () => {
  it("añade, abre, edita y olvida un recuerdo", async () => {
    setBackend(createMockBackend(1, { demoGraph: false }));
    render(<MemoryScreen />);
    fireEvent.change(screen.getByLabelText("Nuevo recuerdo"), { target: { value: "Mi perro se llama Rocky" } });
    fireEvent.click(screen.getByRole("button", { name: "Recordar" }));
    await waitFor(() => expect(screen.getByLabelText("Estado de verificación")).toBeTruthy());
    expect((screen.getByLabelText("Estado de verificación") as HTMLSelectElement).value).toBe("verified");

    fireEvent.change(screen.getByLabelText("Contenido"), { target: { value: "Mi perro se llama Toby" } });
    fireEvent.click(screen.getByRole("button", { name: "Guardar cambios" }));
    await waitFor(() => expect(screen.getAllByText(/Toby/).length).toBeGreaterThan(0));

    fireEvent.click(screen.getByRole("button", { name: "Olvidar" }));
    fireEvent.click(screen.getByRole("button", { name: "Sí, olvidar" }));
    await waitFor(() => expect(screen.getByText(/Elige un recuerdo/)).toBeTruthy());
  });

  it("los filtros por tipo marcan el activo", async () => {
    setBackend(createMockBackend(1));
    render(<MemoryScreen />);
    const entities = await screen.findByRole("button", { name: "Entidades" });
    fireEvent.click(entities);
    expect(entities.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Todos" }).getAttribute("aria-pressed")).toBe("false");
  });

  it("la exportación pide contraseña repetida de al menos 8 caracteres", async () => {
    setBackend(createMockBackend(1));
    render(<MemoryScreen />);
    fireEvent.click(screen.getByRole("tab", { name: "Exportar e importar" }));
    const button = await screen.findByRole("button", { name: "Exportar memoria" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    const [p1, p2] = screen.getAllByLabelText(/Contraseña|Repítela/);
    fireEvent.change(p1, { target: { value: "clave larga 1" } });
    fireEvent.change(p2, { target: { value: "clave larga 2" } });
    expect(screen.getByText("Las contraseñas no coinciden.")).toBeTruthy();
    fireEvent.change(p2, { target: { value: "clave larga 1" } });
    expect((button as HTMLButtonElement).disabled).toBe(false);
  });
});

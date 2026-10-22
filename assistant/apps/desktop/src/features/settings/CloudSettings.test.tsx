import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { createMockBackend, setBackend } from "../../services/backend";
import { CloudSettings } from "./CloudSettings";
import { ModelSettings } from "./ModelSettings";

afterEach(cleanup);

async function pair() {
  await waitFor(() => expect((screen.getByLabelText(/URL del Cloud Core/) as HTMLInputElement).value).toBe("https://jarvis.brix-dev.com"));
  fireEvent.change(screen.getByLabelText(/URL del Cloud Core/), { target: { value: "https://jarvis.example.test" } });
  fireEvent.change(screen.getByLabelText(/Correo del propietario/), { target: { value: "yo@example.test" } });
  fireEvent.change(screen.getByLabelText(/Contraseña/), { target: { value: "secreta-larga" } });
  fireEvent.click(screen.getByRole("button", { name: "Emparejar" }));
  await waitFor(() => expect(screen.getByText(/Equipo emparejado/)).toBeTruthy());
}

describe("Configuración → Infraestructura y Memoria", () => {
  it("en modo local no hay nada sincronizado y el híbrido exige emparejar", async () => {
    setBackend(createMockBackend(0, { demoGraph: false }));
    render(<CloudSettings />);
    await waitFor(() => expect(screen.getByRole("form", { name: "Emparejar con el Cloud Core" })).toBeTruthy());
    expect(screen.getByRole("radio", { name: "Local" }).getAttribute("aria-checked")).toBe("true");
    expect((screen.getByRole("radio", { name: "Híbrido" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Solo en este equipo")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Sincronizar ahora" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("rechaza un Cloud Core sin HTTPS", async () => {
    setBackend(createMockBackend(0, { demoGraph: false }));
    render(<CloudSettings />);
    await waitFor(() => screen.getByRole("form", { name: "Emparejar con el Cloud Core" }));
    fireEvent.change(screen.getByLabelText(/URL del Cloud Core/), { target: { value: "http://jarvis.example.test" } });
    fireEvent.change(screen.getByLabelText(/Correo del propietario/), { target: { value: "yo@example.test" } });
    fireEvent.change(screen.getByLabelText(/Contraseña/), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Emparejar" }));
    await waitFor(() => expect(screen.getByText(/requiere HTTPS/)).toBeTruthy());
  });

  it("emparejar muestra estado, dispositivo, latencia y permite sincronizar y resolver conflictos", async () => {
    setBackend(createMockBackend(0));
    render(<CloudSettings />);
    await waitFor(() => screen.getByRole("form", { name: "Emparejar con el Cloud Core" }));
    await pair();
    expect(screen.getByRole("radio", { name: "Híbrido" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText(/jarvis\.example\.test/)).toBeTruthy();
    expect(screen.getByText(/48 ms/)).toBeTruthy();
    expect(screen.getByText(/PC de escritorio/)).toBeTruthy();
    expect(screen.getByText(/cambio pendiente de enviar/)).toBeTruthy();

    const list = screen.getByRole("list", { name: "Conflictos de sincronización" });
    expect(within(list).getByText(/miércoles a las 9/)).toBeTruthy();
    expect(within(list).getAllByText(/miércoles a las 10/).length).toBeGreaterThan(0);
    fireEvent.click(within(list).getByRole("button", { name: "Quedarme con la del Cloud" }));
    await waitFor(() => expect(screen.queryByRole("list", { name: "Conflictos de sincronización" })).toBeNull());
    expect(screen.getByText("ninguno")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Sincronizar ahora" }));
    await waitFor(() => expect(screen.getByText(/Sincronizado: 1 enviados, 3 recibidos/)).toBeTruthy());
    expect(screen.getByText("Al día")).toBeTruthy();
  });

  it("volver a modo local y desemparejar", async () => {
    setBackend(createMockBackend(0));
    render(<CloudSettings />);
    await waitFor(() => screen.getByRole("form", { name: "Emparejar con el Cloud Core" }));
    await pair();
    fireEvent.click(screen.getByRole("radio", { name: "Local" }));
    await waitFor(() => expect(screen.getByText(/nada sale de este equipo/)).toBeTruthy());
    expect((screen.getByRole("button", { name: "Sincronizar ahora" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Desemparejar" }));
    await waitFor(() => expect(screen.getByRole("form", { name: "Emparejar con el Cloud Core" })).toBeTruthy());
  });
});

describe("Configuración → Modelos", () => {
  it("por defecto usa el servidor; un proveedor propio guarda su clave sin mostrarla", async () => {
    setBackend(createMockBackend(0, { demoGraph: false }));
    render(<ModelSettings />);
    await waitFor(() => screen.getByRole("radiogroup", { name: "Dónde corre el modelo" }));
    expect(screen.getByRole("radio", { name: "Tu servidor" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByLabelText(/^URL/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Probar" }));
    await waitFor(() => expect(screen.getByText(/Empareja la PC/)).toBeTruthy());

    fireEvent.click(screen.getByRole("radio", { name: "Proveedor propio" }));
    fireEvent.click(screen.getByRole("button", { name: "Guardar" }));
    await waitFor(() => expect(screen.getByText(/falta la URL/)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/^URL/), { target: { value: "https://ollama.example.test" } });
    fireEvent.change(screen.getByLabelText(/Clave/), { target: { value: "sk-prueba" } });
    fireEvent.click(screen.getByRole("button", { name: "Guardar" }));
    await waitFor(() => expect(screen.getByText("Proveedor de modelos guardado.")).toBeTruthy());
    const key = screen.getByLabelText(/Clave/) as HTMLInputElement;
    expect(key.value).toBe("");
    expect(key.placeholder).toContain("guardada");
    expect(screen.queryByText(/VibeVoice/)).toBeNull();
  });
});

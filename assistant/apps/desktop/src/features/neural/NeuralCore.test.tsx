import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMockBackend, setBackend } from "../../services/backend";
import { initialGraphState, useGraph } from "../../stores/graphStore";
import { deriveCore, NeuralCore } from "./NeuralCore";

afterEach(() => {
  cleanup();
  useGraph.setState({ ...initialGraphState, positions: new Map() });
});

describe("NeuralCore", () => {
  it("sin WebGL usa la vista 2D y carga recuerdos reales del backend", async () => {
    setBackend(createMockBackend(1));
    render(<NeuralCore height={300} />);
    await waitFor(() => expect(useGraph.getState().nodes.length).toBeGreaterThan(100));
    expect(screen.getByRole("img", { name: /Grafo de conocimiento/ })).toBeTruthy();
    expect(screen.getByText(/sin WebGL: vista 2D/)).toBeTruthy();
    expect(screen.getByText(/recuerdos ·/)).toBeTruthy();
  });

  it("muestra el detalle del nodo elegido", async () => {
    setBackend(createMockBackend(1));
    render(<NeuralCore height={300} />);
    await waitFor(() => expect(useGraph.getState().nodes.length).toBeGreaterThan(0));
    const first = useGraph.getState().nodes[0];
    act(() => useGraph.getState().select(first.id));
    expect(screen.getByRole("complementary", { name: "Recuerdo seleccionado" }).textContent).toContain(first.label);
    fireEvent.click(screen.getByRole("button", { name: "Cerrar" }));
    expect(useGraph.getState().selectedId).toBeNull();
  });

  it("un recuerdo creado en el backend aparece en el grafo por evento", async () => {
    const b = createMockBackend(1, { demoGraph: false });
    setBackend(b);
    const stop = await b.onVisualEvent((e) => useGraph.getState().apply(e));
    render(<NeuralCore height={300} />);
    await waitFor(() => expect(screen.getByText(/Aún no hay recuerdos/)).toBeTruthy());
    await act(async () => {
      await b.addMemory({ content: "Mi proyecto usa FastAPI" });
    });
    expect(useGraph.getState().nodes).toHaveLength(1);
    expect(Object.keys(useGraph.getState().births)).toHaveLength(1);
    stop();
  });

  it("el núcleo refleja escuchar, hablar, pensar y esperar autorización", () => {
    expect(deriveCore("RECORDING_COMMAND", 0.4, 0, false, false, null)).toMatchObject({ listening: true, level: 0.4 });
    expect(deriveCore("SPEAKING", 0.4, 0.7, false, false, null)).toMatchObject({ speaking: true, level: 0.7 });
    expect(deriveCore("IDLE", 0, 0, true, true, 5)).toMatchObject({ busy: true, awaitingApproval: true, flashAt: 5, level: 0 });
  });
});

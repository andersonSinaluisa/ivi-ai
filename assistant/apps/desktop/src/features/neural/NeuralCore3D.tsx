// Neural Core en 3D (React Three Fiber). Todo el grafo se dibuja con tres objetos:
// un `Points` (nodos, shader propio), un `LineSegments` (relaciones) y un `Points` de pulsos.
// No hay un elemento por nodo: los datos viven en buffers que se actualizan en sitio.
import { useEffect, useMemo, useRef } from "react";
import { Canvas, useFrame, useThree, type ThreeEvent } from "@react-three/fiber";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { GraphNode } from "@jarvis/shared-types";
import { KIND_COLOR, type Vec3 } from "./layout";
import type { CoreState, NeuralViewProps } from "./types";
import { birthProgress, FLASH_MS, intensity } from "../../stores/graphStore";

const VERTEX = /* glsl */ `
  attribute float aSize;
  attribute vec3 aColor;
  attribute float aGlow;
  attribute float aPhase;
  uniform float uTime;
  uniform float uPixelRatio;
  uniform float uMotion;
  varying vec3 vColor;
  varying float vGlow;
  void main() {
    vec3 p = position + uMotion * 0.05 * vec3(sin(uTime * 0.6 + aPhase), cos(uTime * 0.5 + aPhase * 1.3), sin(uTime * 0.4 + aPhase * 0.7));
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    gl_PointSize = aSize * (1.0 + aGlow * 1.6) * uPixelRatio * (260.0 / -mv.z);
    gl_Position = projectionMatrix * mv;
    vColor = aColor;
    vGlow = aGlow;
  }
`;

const FRAGMENT = /* glsl */ `
  varying vec3 vColor;
  varying float vGlow;
  void main() {
    float d = length(gl_PointCoord - 0.5);
    if (d > 0.5) discard;
    float core = smoothstep(0.2, 0.0, d);
    float halo = smoothstep(0.5, 0.05, d) * 0.4;
    vec3 col = vColor * (core * 1.1 + halo) + vec3(1.0) * core * vGlow * 0.7;
    gl_FragColor = vec4(col, (core + halo) * (0.6 + 0.4 * vGlow));
  }
`;

const color = new THREE.Color();
const WHITE = new THREE.Color("#e6fbff");
const AMBER = new THREE.Color("#f5b544");
const CYAN = new THREE.Color("#22d3ee");
/** Tono de los recuerdos que llegan del Cloud Core. */
const REMOTE = new THREE.Color("#a78bfa");

function nodeSize(n: GraphNode): number {
  return 0.55 + Math.min(1.2, Math.sqrt(n.degree) * 0.25) + (n.kind === "entity" ? 0.35 : 0);
}

function Controls({ lowPower, target }: { lowPower: boolean; target: Vec3 | null }) {
  const { camera, gl, invalidate } = useThree();
  const controls = useRef<OrbitControls | null>(null);
  useEffect(() => {
    const c = new OrbitControls(camera, gl.domElement);
    c.enableDamping = true;
    c.dampingFactor = 0.08;
    c.minDistance = 3;
    c.maxDistance = 20;
    c.enablePan = false;
    c.addEventListener("change", () => invalidate());
    controls.current = c;
    return () => c.dispose();
  }, [camera, gl, invalidate]);
  useEffect(() => {
    if (controls.current) {
      controls.current.autoRotate = !lowPower && target === null;
      controls.current.autoRotateSpeed = 0.35;
    }
  }, [lowPower, target]);
  useFrame(() => {
    const c = controls.current;
    if (!c) return;
    // La cámara se acerca suavemente al nodo elegido.
    const goal = target ?? [0, 0, 0];
    c.target.x += (goal[0] - c.target.x) * 0.06;
    c.target.y += (goal[1] - c.target.y) * 0.06;
    c.target.z += (goal[2] - c.target.z) * 0.06;
    c.update();
  });
  return null;
}

function Graph(props: NeuralViewProps) {
  const { nodes, edges, positions, version, remote } = props;
  const live = useRef(props);
  live.current = props;
  const { gl, invalidate } = useThree();

  const material = useMemo(
    () =>
      new THREE.ShaderMaterial({
        vertexShader: VERTEX,
        fragmentShader: FRAGMENT,
        uniforms: { uTime: { value: 0 }, uPixelRatio: { value: gl.getPixelRatio() }, uMotion: { value: 1 } },
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    [gl],
  );

  // Buffers: se reconstruyen solo cuando cambia la topología (`version`).
  const { pointGeo, lineGeo, baseColors, sizes, edgeIndex } = useMemo(() => {
    const n = nodes.length;
    const pos = new Float32Array(n * 3);
    const col = new Float32Array(n * 3);
    const size = new Float32Array(n);
    const glow = new Float32Array(n);
    const phase = new Float32Array(n);
    const base = new Float32Array(n * 3);
    const idx = new Map<string, number>();
    nodes.forEach((nd, i) => {
      idx.set(nd.id, i);
      const p = positions.get(nd.id) ?? [0, 0, 0];
      pos.set(p, i * 3);
      color.set(KIND_COLOR[nd.kind]).multiplyScalar(nd.verification === "verified" ? 1 : 0.7);
      if (remote?.[nd.id]) color.lerp(REMOTE, 0.6);
      base.set([color.r, color.g, color.b], i * 3);
      col.set([color.r, color.g, color.b], i * 3);
      size[i] = nodeSize(nd);
      phase[i] = (i * 2.399) % (Math.PI * 2);
    });
    const pg = new THREE.BufferGeometry();
    pg.setAttribute("position", new THREE.BufferAttribute(pos, 3));
    pg.setAttribute("aColor", new THREE.BufferAttribute(col, 3));
    pg.setAttribute("aSize", new THREE.BufferAttribute(size.slice(), 1));
    pg.setAttribute("aGlow", new THREE.BufferAttribute(glow, 1));
    pg.setAttribute("aPhase", new THREE.BufferAttribute(phase, 1));
    pg.computeBoundingSphere();

    const valid = edges.filter((e) => idx.has(e.source) && idx.has(e.target));
    const lpos = new Float32Array(valid.length * 6);
    const lcol = new Float32Array(valid.length * 6);
    valid.forEach((e, i) => {
      const a = positions.get(e.source) ?? [0, 0, 0];
      const b = positions.get(e.target) ?? [0, 0, 0];
      lpos.set([...a, ...b], i * 6);
    });
    const lg = new THREE.BufferGeometry();
    lg.setAttribute("position", new THREE.BufferAttribute(lpos, 3));
    lg.setAttribute("color", new THREE.BufferAttribute(lcol, 3));
    return { pointGeo: pg, lineGeo: lg, baseColors: base, sizes: size, edgeIndex: valid };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);

  useEffect(() => () => pointGeo.dispose(), [pointGeo]);
  useEffect(() => () => lineGeo.dispose(), [lineGeo]);

  const lineMaterial = useMemo(
    () => new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.9, blending: THREE.AdditiveBlending, depthWrite: false }),
    [],
  );

  // Pulsos que viajan por las relaciones iluminadas.
  const MAX_PULSES = 96;
  const pulseGeo = useMemo(() => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(MAX_PULSES * 3), 3));
    g.setDrawRange(0, 0);
    return g;
  }, []);
  const pulseMaterial = useMemo(
    () => new THREE.PointsMaterial({ color: "#e6fbff", size: 0.09, transparent: true, opacity: 0.9, blending: THREE.AdditiveBlending, depthWrite: false }),
    [],
  );

  useFrame((state) => {
    const now = performance.now();
    const p = live.current;
    material.uniforms.uTime.value = state.clock.elapsedTime;
    material.uniforms.uMotion.value = p.lowPower ? 0 : 1;

    // Nodos: brillo por resaltado, tamaño por nacimiento y selección.
    const glow = pointGeo.getAttribute("aGlow") as THREE.BufferAttribute;
    const size = pointGeo.getAttribute("aSize") as THREE.BufferAttribute;
    const col = pointGeo.getAttribute("aColor") as THREE.BufferAttribute;
    let active = false;
    nodes.forEach((nd, i) => {
      const h = intensity(p.highlights[nd.id], now);
      const b = birthProgress(p.births[nd.id], now);
      const selected = p.selectedId === nd.id;
      if (h > 0 || b < 1) active = true;
      glow.setX(i, Math.max(h, selected ? 0.8 : 0));
      // Nacimiento con un pequeño rebote.
      const grow = b < 1 ? b * (1 + Math.sin(b * Math.PI) * 0.6) : 1;
      size.setX(i, sizes[i] * grow * (selected ? 1.5 : 1));
      const mix = h * 0.6;
      col.setXYZ(
        i,
        baseColors[i * 3] * (1 - mix) + WHITE.r * mix,
        baseColors[i * 3 + 1] * (1 - mix) + WHITE.g * mix,
        baseColors[i * 3 + 2] * (1 - mix) + WHITE.b * mix,
      );
    });
    glow.needsUpdate = size.needsUpdate = col.needsUpdate = true;

    // Relaciones: tenues según confianza; las inferidas sin verificar, más tenues aún.
    const lc = lineGeo.getAttribute("color") as THREE.BufferAttribute;
    const pulses = pulseGeo.getAttribute("position") as THREE.BufferAttribute;
    let pulseCount = 0;
    edgeIndex.forEach((e, i) => {
      const h = Math.max(intensity(p.highlights[e.id], now), 0.5 * Math.min(intensity(p.highlights[e.source], now), intensity(p.highlights[e.target], now)));
      const touchesSelected = p.selectedId !== null && (e.source === p.selectedId || e.target === p.selectedId);
      const baseAlpha = (0.08 + e.confidence * 0.18) * (e.provenance === "inferred" && !e.verified ? 0.55 : 1);
      const a = Math.max(baseAlpha, h * 0.9, touchesSelected ? 0.6 : 0);
      color.set(KIND_COLOR.semantic).lerp(WHITE, h);
      for (let k = 0; k < 2; k++) lc.setXYZ(i * 2 + k, color.r * a, color.g * a, color.b * a);
      if (h > 0.05 && pulseCount < MAX_PULSES) {
        active = true;
        const t = ((now / 900) + i * 0.37) % 1;
        const A = positions.get(e.source);
        const B = positions.get(e.target);
        if (A && B) {
          pulses.setXYZ(pulseCount++, A[0] + (B[0] - A[0]) * t, A[1] + (B[1] - A[1]) * t, A[2] + (B[2] - A[2]) * t);
        }
      }
    });
    lc.needsUpdate = true;
    pulses.needsUpdate = true;
    pulseGeo.setDrawRange(0, pulseCount);

    // En bajo consumo solo se piden fotogramas mientras hay animación.
    if (p.lowPower && active) invalidate();
  });

  const pick = (e: ThreeEvent<MouseEvent>) => {
    e.stopPropagation();
    if (e.index !== undefined && nodes[e.index]) props.onSelect(nodes[e.index].id);
  };

  return (
    <group>
      <lineSegments geometry={lineGeo} material={lineMaterial} />
      <points geometry={pointGeo} material={material} onClick={pick} />
      <points geometry={pulseGeo} material={pulseMaterial} />
    </group>
  );
}

/** Núcleo central: el estado del asistente (escuchar, pensar, hablar, esperar autorización). */
function Core({ core, lowPower }: { core: CoreState; lowPower: boolean }) {
  const sphere = useRef<THREE.Mesh>(null);
  const ring = useRef<THREE.Mesh>(null);
  const live = useRef(core);
  live.current = core;
  const smooth = useRef(0);
  const { invalidate } = useThree();
  useFrame((state) => {
    const c = live.current;
    const t = state.clock.elapsedTime;
    smooth.current += (c.level - smooth.current) * 0.25;
    let scale = 1;
    if (c.speaking) scale = 1 + smooth.current * 0.9;
    else if (c.listening) scale = 1 + 0.12 * (0.5 + 0.5 * Math.sin(t * 4)) + smooth.current * 0.5;
    else if (c.busy && !lowPower) scale = 1 + 0.06 * Math.sin(t * 2.4);
    if (sphere.current) {
      sphere.current.scale.setScalar(scale);
      const m = sphere.current.material as THREE.MeshBasicMaterial;
      m.color.copy(c.awaitingApproval ? AMBER : CYAN);
      m.opacity = c.busy || c.listening || c.speaking ? 0.95 : 0.55;
    }
    if (ring.current) {
      const f = c.flashAt === null ? 1 : (performance.now() - c.flashAt) / FLASH_MS;
      const visible = f >= 0 && f < 1;
      ring.current.visible = visible;
      if (visible) {
        ring.current.scale.setScalar(1 + f * 6);
        (ring.current.material as THREE.MeshBasicMaterial).opacity = 0.6 * (1 - f);
        ring.current.lookAt(state.camera.position);
      }
      if (lowPower && (visible || c.listening || c.speaking)) invalidate();
    }
  });
  return (
    <group>
      <mesh ref={sphere}>
        <icosahedronGeometry args={[0.32, lowPower ? 1 : 3]} />
        <meshBasicMaterial color="#22d3ee" transparent opacity={0.6} blending={THREE.AdditiveBlending} depthWrite={false} />
      </mesh>
      <mesh ref={ring} visible={false}>
        <ringGeometry args={[0.36, 0.4, 64]} />
        <meshBasicMaterial color="#e6fbff" transparent opacity={0} side={THREE.DoubleSide} blending={THREE.AdditiveBlending} depthWrite={false} />
      </mesh>
    </group>
  );
}

export function NeuralCore3D(props: NeuralViewProps) {
  const target = props.selectedId ? (props.positions.get(props.selectedId) ?? null) : null;
  return (
    <Canvas
      dpr={props.lowPower ? 1 : [1, 1.75]}
      frameloop={props.lowPower ? "demand" : "always"}
      gl={{ antialias: !props.lowPower, powerPreference: props.lowPower ? "low-power" : "high-performance" }}
      camera={{ position: [0, 1.5, 10], fov: 50 }}
      raycaster={{ params: { Points: { threshold: 0.2 }, Line: { threshold: 0 }, Mesh: {}, LOD: {}, Sprite: {} } }}
      onPointerMissed={() => props.onSelect(null)}
      aria-label="Grafo de conocimiento de Jarvis en 3D"
    >
      <color attach="background" args={["#030814"]} />
      {!props.lowPower && <fog attach="fog" args={["#030814", 10, 24]} />}
      <Controls lowPower={props.lowPower} target={target} />
      <Graph {...props} />
      <Core core={props.core} lowPower={props.lowPower} />
    </Canvas>
  );
}

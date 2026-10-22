// jsdom no implementa canvas: se comporta como un equipo sin WebGL ni Canvas 2D.
HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;

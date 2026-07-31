import game from '/classic/state.js';
import { Tilemap, IsometricNavMesh, IsoAgent } from '/classic/isometric.js';
import { Rectangle, Sprite, Text } from '/classic/transforms.js';
import { Collider, Polygon } from '/classic/collision.js';

import { vec2, vec3 } from 'gl-matrix';

// Extend game state for editor target
declare module './types.js' {
  interface IGameState {
    editorTarget?: string;
  }
}

export function initCursor(): void {
  const cursor = game.getEntity('cursor')!;
  const compSprite = cursor.getComponent(Sprite)!;

  cursor.registerCall('update', function () {
    vec3.copy(compSprite.position, game.mousePos);
  });
}

export function initFPSCounter(): void {
  const fpsCounter = game.getEntity('fpsCounter')!;
  const compFPSText = fpsCounter.getComponent(Text)!;

  let timeAccumulator = 0;
  let lastFPS = 0;

  fpsCounter.registerCall('update', function () {
    timeAccumulator += game.deltaTime;
    if (timeAccumulator >= 0.2) {
      lastFPS = game.fps;
      compFPSText.setText(lastFPS.toString());
      timeAccumulator = 0;
    }
  });
}

export function initInfoText(): void {
  const label1 = game.getEntity('textLabel1')!;
  const label1Text = label1.getComponent(Text)!;
  label1Text.setText('CLASSIC ENGINE V0.1A0 ;)');

  const label2 = game.getEntity('textLabel2')!;
  const label2Text = label2.getComponent(Text)!;
  label2Text.setText('SCROLL WHEEL TO ZOOM, WASD TO MOVE CAM');
}

export function initCameraControllerWASD(): void {
  const camController = game.getEntity('camController')!;
  camController.registerCall('update', function () {
    if (game.isKeyDown('KeyW'))
      game.camera.position[1] -= game.scrollSpeed * game.deltaTime;
    if (game.isKeyDown('KeyS'))
      game.camera.position[1] += game.scrollSpeed * game.deltaTime;
    if (game.isKeyDown('KeyA'))
      game.camera.position[0] -= game.scrollSpeed * game.deltaTime;
    if (game.isKeyDown('KeyD'))
      game.camera.position[0] += game.scrollSpeed * game.deltaTime;

    if (Math.abs(game.mouseWheel) > 0.01) {
      game.camera.scale[0] += game.mouseWheel * game.deltaTime;
      game.camera.scale[1] += game.mouseWheel * game.deltaTime;

      vec3.max(game.camera.scale as vec3, game.camera.scale as vec3, [0.1, 0.1, 1]);
    }
  });
}

export function initTilemap(): void {
  const tilemap = game.getEntity('tilemap')!;
  const compTilemap = tilemap.getComponent(Tilemap)!;

  const tilemapVerts: vec3[] = [
    vec3.fromValues(0, 0, 0),
    vec3.fromValues(1, 0, 0),
    vec3.fromValues(1, 1, 0),
    vec3.fromValues(0, 1, 0),
  ];
  for (let i = 0; i < tilemapVerts.length; i++) {
    compTilemap.isoToCartesian(tilemapVerts[i]);
  }

  const compTilemapCollider = tilemap.addComponent(
    Collider,
    new Polygon(game, [0, 0, 0], [1, 1, 1], 0, tilemapVerts)
  ) as Collider;

  tilemap.registerCall('update', function () {
    const camDelta = game.camera.getFix();
    vec3.negate(camDelta, camDelta);
    vec3.copy(compTilemapCollider.position, camDelta);

    vec3.copy(compTilemapCollider.scale, [
      compTilemap.mapSize[0] * game.camera.scale[0],
      compTilemap.mapSize[1] * game.camera.scale[1],
      1,
    ]);
    compTilemapCollider.updateRect();
  });
}

export function initSelectionMonitor(): void {
  (game as typeof game & { editorTarget: string }).editorTarget = 'none';

  const tileSelector = game.getEntity('tilemapTileSelector')!;
  const tilemapEditor = game.getEntity('tilemapEditor')!;

  const navMesh = game.getEntity('tilemapNavigation')!;
  const navMeshSelector = game.getEntity('navMeshTileSelector')!;
  const navMeshEditor = game.getEntity('navMeshEditor')!;

  const monitor = game.getEntity('selectionMonitor')!;
  monitor.registerCall('update', function () {
    tileSelector.enabled = game.editorTarget === 'tilemap';
    tilemapEditor.enabled = game.editorTarget === 'tilemap';

    navMesh.enabled = game.editorTarget === 'navMesh';
    navMeshSelector.enabled = game.editorTarget === 'navMesh';
    navMeshEditor.enabled = game.editorTarget === 'navMesh';
  });
}

const rectVerts: [number, number, number][] = [
  [0, 0, 0],
  [1, 0, 0],
  [1, 1, 0],
  [0, 1, 0],
];

export function initTilemapEditor(): void {
  const tilemap = game.getEntity('tilemap')!;
  const tileSelector = game.getEntity('tilemapTileSelector')!;
  const tilemapEditor = game.getEntity('tilemapEditor')!;

  const compTilemap = tilemap.getComponent(Tilemap)!;
  const compTilemapCollider = tilemap.getComponent(Collider)!;

  const uiBorder = 10;
  let selectedTile = 0;
  const localPos = vec3.fromValues(0, 0, 0);

  const compTilemapSprite = tilemapEditor.getComponent(Sprite)!;
  const compTilemapSpriteBG = tilemapEditor.getComponent(Rectangle)!;

  const compTileSelector = tileSelector.getComponent(Rectangle)!;

  const editorCollider = tilemapEditor.addComponent(
    Collider,
    new Polygon(
      game,
      [0, 0, 0],
      [...compTilemap.tileSetPixelSize, 1],
      0,
      rectVerts
    )
  ) as Collider;

  editorCollider.addHandler('click', function () {
    const mouseLocal = vec3.clone(game.mousePos);
    vec3.sub(mouseLocal, mouseLocal, compTilemapSprite.position);

    localPos[0] = Math.floor(mouseLocal[0] / compTilemap.tilePixelSize[0]);
    localPos[1] = Math.floor(mouseLocal[1] / compTilemap.tilePixelSize[1]);
    localPos[2] = 0;

    selectedTile = Math.min(
      compTilemap.maxTile,
      localPos[0] + localPos[1] * compTilemap.tileSetSize[0]
    );

    return true;
  });

  tilemapEditor.registerCall('update', function () {
    compTilemapSprite.position = vec3.fromValues(
      game.canvas!.width - compTilemap.tileSetPixelSize[0] - uiBorder,
      game.canvas!.height - compTilemap.tileSetPixelSize[1] - uiBorder,
      compTilemapSprite.position[2]
    );

    vec3.copy(compTilemapSpriteBG.position, compTilemapSprite.position);
    compTilemapSpriteBG.scale = vec3.fromValues(
      compTilemap.tileSetPixelSize[0],
      compTilemap.tileSetPixelSize[1],
      1
    );

    compTileSelector.position = vec3.fromValues(
      compTilemapSprite.position[0] + localPos[0] * compTilemap.tilePixelSize[0],
      compTilemapSprite.position[1] + localPos[1] * compTilemap.tilePixelSize[1],
      compTileSelector.position[2]
    );

    vec3.copy(editorCollider.position, compTilemapSprite.position);
    editorCollider.updateRect();
  });

  compTilemapCollider.addHandler('selection', function () {
    if (game.editorTarget !== 'tilemap') return;
    const [begin, end] = compTilemap.getSelection();

    vec2.max(begin, begin, [0, 0]);
    vec2.min(end, end, compTilemap.mapSize as vec2);

    compTilemap.fillRegion(begin, end, selectedTile);
    compTilemap.uploadToGPU();
  });
}

export async function initNavMeshEditor(): Promise<void> {
  const navMesh = game.getEntity('tilemapNavigation')!;
  const compNavMesh = navMesh.getComponent(IsometricNavMesh)!;

  const tilemap = game.getEntity('tilemap')!;
  const compTilemapCollider = tilemap.getComponent(Collider)!;

  const navMeshSelector = game.getEntity('navMeshTileSelector')!;
  const navMeshEditor = game.getEntity('navMeshEditor')!;

  const uiBorder = 10;
  const uiScale = 4;
  let selectedTile = 0;
  const localPos = vec3.fromValues(0, 0, 0);

  const compTilemapSprite = navMeshEditor.getComponent(Sprite)!;
  const compTilemapSpriteBG = navMeshEditor.getComponent(Rectangle)!;

  const compTileSelector = navMeshSelector.getComponent(Rectangle)!;

  compTileSelector.scale = vec3.fromValues(
    compNavMesh.tilePixelSize[0] * uiScale,
    compNavMesh.tilePixelSize[1] * uiScale,
    1
  );
  compTilemapSprite.scale = vec3.fromValues(uiScale, uiScale, 1);
  compTilemapSpriteBG.scale = vec3.fromValues(uiScale, uiScale, 1);

  const editorCollider = navMeshEditor.addComponent(
    Collider,
    new Polygon(
      game,
      [0, 0, 0],
      [
        compNavMesh.tileSetPixelSize[0] * uiScale,
        compNavMesh.tileSetPixelSize[1] * uiScale,
        1,
      ],
      0,
      rectVerts
    )
  ) as Collider;

  editorCollider.addHandler('click', function () {
    const mouseLocal = vec3.clone(game.mousePos);
    vec3.sub(mouseLocal, mouseLocal, compTilemapSprite.position);

    localPos[0] = Math.floor(mouseLocal[0] / (compNavMesh.tilePixelSize[0] * uiScale));
    localPos[1] = Math.floor(mouseLocal[1] / (compNavMesh.tilePixelSize[1] * uiScale));
    localPos[2] = 0;

    selectedTile = Math.min(
      compNavMesh.maxTile,
      localPos[0] + localPos[1] * (compNavMesh.tileSetSize[0] * uiScale)
    );

    return true;
  });

  navMeshEditor.registerCall('update', function () {
    compTilemapSprite.position = vec3.fromValues(
      game.canvas!.width - compNavMesh.tileSetPixelSize[0] * uiScale - uiBorder,
      game.canvas!.height - compNavMesh.tileSetPixelSize[1] * uiScale - uiBorder,
      compTilemapSprite.position[2]
    );

    vec3.copy(compTilemapSpriteBG.position, compTilemapSprite.position);
    compTilemapSpriteBG.scale = vec3.fromValues(
      compNavMesh.tileSetPixelSize[0] * uiScale,
      compNavMesh.tileSetPixelSize[1] * uiScale,
      1
    );

    compTileSelector.position = vec3.clone(compTilemapSprite.position);
    vec3.add(compTileSelector.position, compTileSelector.position, [
      localPos[0] * compNavMesh.tilePixelSize[0] * uiScale,
      localPos[1] * compNavMesh.tilePixelSize[1] * uiScale,
      0,
    ]);

    vec3.copy(editorCollider.position, compTilemapSprite.position);
    editorCollider.updateRect();
  });

  compTilemapCollider.addHandler('selection', function () {
    if (game.editorTarget !== 'navMesh') return;
    const [begin, end] = compNavMesh.getSelection();

    vec2.max(begin, begin, [0, 0]);
    vec2.min(end, end, compNavMesh.mapSize as vec2);

    compNavMesh.fillRegion(begin, end, selectedTile);
    compNavMesh.uploadToGPU();

    compNavMesh.updateMap(
      [0, 0],
      [compNavMesh.sizeX, compNavMesh.sizeY],
      compNavMesh.data
    );
  });
}

export function initAgent(): void {
  const tilemap = game.getEntity('tilemap')!.getComponent(Tilemap)!;
  const navAgent = game.getEntity('navAgent')!;
  const agent = navAgent.getComponent(IsoAgent)!;
  const pathfinder = game.getEntity('tilemapNavigation')!.getComponent(IsometricNavMesh)!;

  const collider = navAgent.addComponent(
    Collider,
    new Polygon(game, [0, 0, 0], [1, 1, 1], 0, rectVerts)
  ) as Collider;

  collider.addHandler('enter', function () {
    // collider.debugColor = [1, 1, 1, .2];
  });

  collider.addHandler('exit', function () {
    // collider.debugColor = [.1, .1, .1, .2];
  });

  navAgent.registerCall('update', function () {
    const cPos = vec3.clone(agent.position);
    tilemap.isoToCartesian(cPos);

    cPos[0] -= agent.tilePixelSize[0] * (agent.anchor[0] as number);
    cPos[1] -= agent.tilePixelSize[1] * (agent.anchor[1] as number);
    cPos[0] *= game.camera.scale[0];
    cPos[1] *= game.camera.scale[1];
    vec3.sub(cPos, cPos, game.camera.getFix());

    const scale: vec3 = [
      agent.tilePixelSize[0] * game.camera.scale[0],
      agent.tilePixelSize[1] * game.camera.scale[1],
      1,
    ];
    vec3.copy(collider.position, cPos);
    vec3.copy(collider.scale, scale);
    collider.updateRect();
  });

  const compTilemapCollider = tilemap.entity.getComponent(Collider)!;

  compTilemapCollider.addHandler('click', function () {
    const start: [number, number] = [agent.position[0], agent.position[1]];
    const end: [number, number] = [tilemap.mouseIsoPos[0], tilemap.mouseIsoPos[1]];

    if (vec2.dist(start, end) > 2) {
      pathfinder.findPath(start, end).then((p) => {
        if (p != null) agent.followPath(p as [number, number][]);
      });
    }
  });
}

export function initDEVButtons(): void {
  const centeredRectVerts: [number, number, number][] = [
    [-0.5, -0.5, 0],
    [0.5, -0.5, 0],
    [0.5, 0.5, 0],
    [-0.5, 0.5, 0],
  ];

  const btnDEVScale = 0.5;
  const btnTilemapScale = 0.25;
  const btnNavMeshScale = 0.25;
  const wiggleFactor = 24;
  const btnPixelSize = [64, 64];

  const btnToolTilemap = game.getEntity('btnTilemap')!;
  const btnTilemap = btnToolTilemap.getComponent(Sprite)!;
  const btnTilemapCollider = btnToolTilemap.addComponent(
    Collider,
    new Polygon(game, [0, 0, 0], [btnPixelSize[0], btnPixelSize[1], 1], 0, centeredRectVerts)
  ) as Collider;

  btnTilemapCollider.addHandler('click', function () {
    (game as typeof game & { editorTarget: string }).editorTarget = 'tilemap';
    return true;
  });

  const btnToolNavMesh = game.getEntity('btnNavMesh')!;
  const btnNavMesh = btnToolNavMesh.getComponent(Sprite)!;
  const btnNavMeshCollider = btnToolNavMesh.addComponent(
    Collider,
    new Polygon(game, [0, 0, 0], [btnPixelSize[0], btnPixelSize[1], 1], 0, centeredRectVerts)
  ) as Collider;

  btnNavMeshCollider.addHandler('click', function () {
    (game as typeof game & { editorTarget: string }).editorTarget = 'navMesh';
    return true;
  });

  const btnDEVEntity = game.getEntity('btnDEV')!;
  const btnDEV = btnDEVEntity.getComponent(Sprite)!;

  const verts: vec3[] = [
    vec3.fromValues(0.1, 0.15, 0),
    vec3.fromValues(0.35, 0.1, 0),
    vec3.fromValues(0.92, 0.55, 0),
    vec3.fromValues(0.94, 0.85, 0),
    vec3.fromValues(0.68, 0.88, 0),
    vec3.fromValues(0.06, 0.45, 0),
  ];
  for (let i = 0; i < verts.length; i++) {
    vec3.sub(verts[i], verts[i], [0.5, 0.5, 0]);
  }

  const btnDEVCollider = btnDEVEntity.addComponent(
    Collider,
    new Polygon(game, [0, 0, 0], [btnPixelSize[0] * 2, btnPixelSize[1] * 2, 1], 0, verts)
  ) as Collider;

  let sineCounter = 0;
  let timeSinceClick = 10;

  let targetTools = btnDEV.position[0] - 300;
  let startPos = btnDEV.position[0];

  btnDEVCollider.addHandler('click', function () {
    timeSinceClick = 0;
    if (targetTools === btnDEV.position[0]) {
      targetTools = btnDEV.position[0] - 300;
      startPos = btnDEV.position[0];
      (game as typeof game & { editorTarget: string }).editorTarget = 'none';
    } else {
      targetTools = btnDEV.position[0];
      startPos = btnDEV.position[0] - 300;
    }
    return true;
  });

  btnDEVEntity.registerCall('update', function () {
    const canvasHeight = game.canvas!.height;
    const wiggle = Math.sin(Math.PI * sineCounter) / wiggleFactor;
    const clickPulse =
      timeSinceClick < 0.8 ? Math.sin((timeSinceClick + Math.PI / 4) * 2) / 8 : 0;

    btnDEV.position = vec3.fromValues(
      btnPixelSize[0],
      canvasHeight - btnPixelSize[1],
      btnDEV.position[2]
    );

    const yTilemap = canvasHeight - btnPixelSize[1] * 3;
    const yNavMesh = canvasHeight - btnPixelSize[1] * 4;

    if (timeSinceClick <= 1) {
      vec3.lerp(
        btnTilemap.position,
        [startPos, yTilemap, btnTilemap.position[2]],
        [targetTools, yTilemap, btnTilemap.position[2]],
        timeSinceClick
      );
      vec3.lerp(
        btnNavMesh.position,
        [startPos, yNavMesh, btnNavMesh.position[2]],
        [targetTools, yNavMesh, btnNavMesh.position[2]],
        timeSinceClick
      );
    } else {
      btnTilemap.position = vec3.fromValues(targetTools, yTilemap, btnTilemap.position[2]);
      btnNavMesh.position = vec3.fromValues(targetTools, yNavMesh, btnNavMesh.position[2]);
    }

    // Update collider positions
    const pairs: [Sprite, Collider][] = [
      [btnDEV, btnDEVCollider],
      [btnTilemap, btnTilemapCollider],
      [btnNavMesh, btnNavMeshCollider],
    ];

    for (const [btn, collider] of pairs) {
      vec3.copy(collider.position, btn.position);
      collider.updateRect();
    }

    // Button interactivity
    const applyScale = (btn: Sprite, collider: Collider, baseScale: number) => {
      if (game.physics!.gjk(collider, game.physics!.mouse)) {
        btn.scale[0] = baseScale + wiggle + clickPulse;
        btn.scale[1] = baseScale + wiggle + clickPulse;
      } else {
        btn.scale[0] = baseScale;
        btn.scale[1] = baseScale;
      }
    };

    applyScale(btnDEV, btnDEVCollider, btnDEVScale);
    applyScale(btnTilemap, btnTilemapCollider, btnTilemapScale);
    applyScale(btnNavMesh, btnNavMeshCollider, btnNavMeshScale);

    sineCounter = (sineCounter + game.deltaTime) % 1;
    timeSinceClick += game.deltaTime * 3;
  });
}

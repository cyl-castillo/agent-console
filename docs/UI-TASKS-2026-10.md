# Backlog de interfaz — octubre de 2026

Este backlog propone mejoras de interfaz, no cambios ya implementados. En
particular, no reabre los trabajos cerrados de tokens de foco, wrapper de
modales, notificaciones, re-run desde el composer, aprobaciones por teclado ni
la agrupación del workbench.

Convenciones: P1 = siguiente prioridad, P2 = importante, P3 = deseable. Los
tamaños son S (hasta un día), M (varios días) y L (más de una iteración).

## Quick wins

### U1 — Hacer operables las salas guardadas desde teclado

- **Problema.** Cada sala guardada se abre mediante un `<li>` con `onClick`, no
  con un control enfocable, y el botón de borrado sólo se identifica por el
  `title` y el glifo `×` ([`src/components/RoomsList.tsx:91`](../src/components/RoomsList.tsx#L91),
  [`src/components/RoomsList.tsx:99`](../src/components/RoomsList.tsx#L99)).
- **Cambio propuesto.** Convertir la apertura en un botón o enlace semántico;
  conservar el borrado como acción separada con nombre accesible que incluya el
  nombre de la sala.
- **Aceptación.** Tab permite abrir y borrar cada sala, Enter/Espacio abre la
  sala enfocada, el lector de pantalla anuncia ambas acciones y borrar no abre
  accidentalmente la sala.
- **Tamaño / prioridad.** S / P1.

### U2 — Exponer un lanzador visible para el composer

- **Problema.** `Composer` sólo se monta después del evento
  `ac:toggle-composer` ([`src/App.tsx:323`](../src/App.tsx#L323),
  [`src/App.tsx:804`](../src/App.tsx#L804)); el propio componente explica sus
  atajos únicamente una vez abierto ([`src/components/Composer.tsx:73`](../src/components/Composer.tsx#L73)).
- **Cambio propuesto.** Añadir en el área Terminal un botón persistente
  “Redactar prompt”, con atajo visible y texto contextual cuando no haya una
  sesión activa.
- **Aceptación.** Una persona puede encontrar y abrir el editor sin conocer la
  paleta ni un atajo; el botón anuncia su estado abierto/cerrado y no intenta
  enviar a una sesión inexistente.
- **Tamaño / prioridad.** S / P2.

### U3 — Permitir reintentar el índice fallido de la paleta

- **Problema.** La paleta muestra el fallo de indexación como texto estático
  ([`src/components/CommandPalette.tsx:90`](../src/components/CommandPalette.tsx#L90)); no ofrece recuperación desde esa vista.
- **Cambio propuesto.** Añadir un botón “Reintentar” que reinicie el índice, con
  estados claros de indexando, fallo y resultado vacío.
- **Aceptación.** Tras simular un error, el usuario puede reintentar sin cerrar
  la paleta; el botón queda deshabilitado durante la petición y el resultado se
  anuncia sin duplicar mensajes.
- **Tamaño / prioridad.** S / P2.

## Consistencia visual

### U4 — Terminar de tokenizar los colores de la experiencia Room y la paleta

- **Problema.** Aunque el tema define escalas de color
  ([`src/styles/global.css:3`](../src/styles/global.css#L3)), Room mantiene una
  paleta hexadecimal e inyecta colores inline
  ([`src/components/RoundtablePanel.tsx:20`](../src/components/RoundtablePanel.tsx#L20),
  [`src/components/RoundtablePanel.tsx:393`](../src/components/RoundtablePanel.tsx#L393)); la paleta de comandos también conserva colores y fondos fijos
  ([`src/styles/global.css:6629`](../src/styles/global.css#L6629),
  [`src/styles/global.css:6681`](../src/styles/global.css#L6681)). Esto puede
  degradar contraste y coherencia en tema claro.
- **Cambio propuesto.** Definir tokens semánticos para participantes, tipos de
  resultado, selección y error; consumirlos desde CSS (incluidos los estados de
  Room) y comprobar ambos temas.
- **Aceptación.** No quedan colores hex/rgba de estado nuevos en esos dos
  flujos; los estados activo, error y selección son distinguibles y superan
  contraste AA para texto normal en oscuro y claro.
- **Tamaño / prioridad.** M / P2.

### U5 — Establecer una escala tipográfica mínima legible en navegación densa

- **Problema.** La barra del workbench usa etiquetas de 10 px y badges de 9 px
  ([`src/styles/global.css:6930`](../src/styles/global.css#L6930),
  [`src/styles/global.css:6936`](../src/styles/global.css#L6936)); la barra de
  estado mide sólo 22 px de alto y texto de 11 px
  ([`src/styles/global.css:7056`](../src/styles/global.css#L7056)). La densidad
  perjudica lectura y objetivos táctiles.
- **Cambio propuesto.** Definir tokens de tipografía y altura interactiva para
  navegación compacta, y aplicarlos primero a workbench, barra de estado y
  secciones laterales.
- **Aceptación.** Las etiquetas y contadores se leen a 100 % de escala sin
  zoom; los controles interactivos alcanzan 24×24 px o tienen una zona de
  puntero equivalente; no aparece scroll horizontal a 1280 px.
- **Tamaño / prioridad.** M / P2.

## Estados y feedback

### U6 — Diferenciar carga, vacío y error en el lateral

- **Problema.** El árbol muestra “Loading…” siempre que `tree` es nulo
  ([`src/components/LeftSidebar.tsx:108`](../src/components/LeftSidebar.tsx#L108)),
  y la lista de salas sólo distingue “sin salas” de contenido
  ([`src/components/RoomsList.tsx:40`](../src/components/RoomsList.tsx#L40)).
  Un error de carga puede parecer una espera indefinida o un resultado vacío.
- **Cambio propuesto.** Modelar `loading`, `empty` y `error` en árbol y salas,
  con causa breve y CTA de reintento cuando corresponda.
- **Aceptación.** Con respuestas de carga, vacías y fallidas simuladas, cada
  área presenta texto distinto; error ofrece reintento y un lector de pantalla
  recibe el cambio de estado.
- **Tamaño / prioridad.** M / P1.

### U7 — Confirmar el fallo de envío del composer

- **Problema.** Si `typeIntoActiveSession` devuelve `false`, `send` no cambia
  el estado ni informa al usuario ([`src/components/Composer.tsx:39`](../src/components/Composer.tsx#L39)).
- **Cambio propuesto.** Conservar el borrador, mostrar un error local accionable
  (“elige o inicia una sesión activa”) y dar foco al control relevante.
- **Aceptación.** Forzando un envío sin sesión válida, el texto no se pierde,
  aparece un mensaje visible y anunciado, y al resolver la condición se puede
  reenviar sin copiar el contenido.
- **Tamaño / prioridad.** S / P1.

### U8 — Mostrar progreso y recuperación en conversaciones Room

- **Problema.** Durante un turno sin eventos, Room sólo muestra “starting
  turn…” ([`src/components/RoundtablePanel.tsx:392`](../src/components/RoundtablePanel.tsx#L392)); los errores se reducen a un banner que puede quedar fuera del
  contexto del turno ([`src/components/RoundtablePanel.tsx:413`](../src/components/RoundtablePanel.tsx#L413)).
- **Cambio propuesto.** Mostrar fase, agente, tiempo transcurrido y última
  actividad en un estado persistente; para error, ofrecer acciones claras de
  reintentar/continuar/detener según la fase.
- **Aceptación.** Una conversación lenta explica quién está trabajando y desde
  cuándo; un fallo no deja al usuario sin siguiente paso; las acciones no se
  ofrecen en una sala guardada de sólo lectura.
- **Tamaño / prioridad.** M / P2.

## Navegación y descubribilidad

### U9 — Dar una alternativa compacta al rail vertical del workbench

- **Problema.** El rail tiene diez grupos más “Modules” en sólo 64 px de ancho
  y habilita scroll vertical ([`src/components/WorkbenchTabs.tsx:157`](../src/components/WorkbenchTabs.tsx#L157),
  [`src/styles/global.css:6876`](../src/styles/global.css#L6876)). En ventanas bajas, grupos y contadores quedan fuera de vista sin indicar que hay más.
- **Cambio propuesto.** Diseñar un modo compacto para altura limitada: menú
  “Más”, indicador de overflow y acceso conservado al grupo activo; definir
  puntos de corte por altura, no sólo por anchura.
- **Aceptación.** A 900×600 se puede llegar a todos los grupos sin descubrirlos
  accidentalmente por scroll; el grupo activo permanece visible y los badges no
  quedan recortados.
- **Tamaño / prioridad.** M / P2.

### U10 — Hacer que la barra de estado degrade con elegancia

- **Problema.** `StatusBar` añade en una sola fila rama, cambios, sesión,
  estado del agente, memoria, modelo, uso, voz, hooks, metadatos y versión
  ([`src/components/StatusBar.tsx:50`](../src/components/StatusBar.tsx#L50));
  su CSS no define overflow ni prioridades de ocultación
  ([`src/styles/global.css:7056`](../src/styles/global.css#L7056)).
- **Cambio propuesto.** Establecer prioridades responsive: conservar bloqueo,
  sesión y cambios; agrupar detalles secundarios en un menú o tooltip, con
  truncado controlado.
- **Aceptación.** Entre 900 y 1440 px no hay solapamiento ni scroll horizontal;
  “waiting”, cambios y sesión activa siguen visibles; los datos ocultos siguen
  consultables por teclado.
- **Tamaño / prioridad.** M / P2.

### U11 — Reducir la fragilidad visual del panel Room en 240 px

- **Problema.** El panel derecho parte de 320 px pero puede reducirse a 240 px
  ([`src/App.tsx:82`](../src/App.tsx#L82), [`src/App.tsx:255`](../src/App.tsx#L255));
  Room combina roster, metadatos, presupuesto, transcript, connector y acciones
  en el mismo ancho ([`src/components/RoundtablePanel.tsx:321`](../src/components/RoundtablePanel.tsx#L321),
  [`src/components/RoundtablePanel.tsx:679`](../src/components/RoundtablePanel.tsx#L679)).
- **Cambio propuesto.** Probar el flujo en 240 px y crear variantes apiladas o
  plegables para metadatos, roster y delegaciones largas.
- **Aceptación.** En 240 px y 320 px no se corta texto crítico ni se solapan
  botones; instrucciones largas se pueden leer o expandir; transcript conserva
  scroll independiente.
- **Tamaño / prioridad.** M / P2.

## Accesibilidad

### U12 — Convertir la paleta de comandos en un combobox/listbox accesible

- **Problema.** Los resultados seleccionables son `<div>` clicables sin rol,
  foco ni semántica de selección ([`src/components/CommandPalette.tsx:101`](../src/components/CommandPalette.tsx#L101)); el input tampoco expone una etiqueta accesible
  ([`src/components/CommandPalette.tsx:80`](../src/components/CommandPalette.tsx#L80)).
- **Cambio propuesto.** Implementar el patrón ARIA combobox/listbox: etiqueta,
  `aria-expanded`, `aria-controls`, opción activa mediante
  `aria-activedescendant` y estados de carga/error anunciados.
- **Aceptación.** NVDA/VoiceOver anuncia consulta, cantidad/estado y opción
  activa al usar flechas; Enter ejecuta la opción anunciada; ratón y teclado
  conservan el mismo resultado.
- **Tamaño / prioridad.** M / P1.

### U13 — Ofrecer redimensionado de paneles sin puntero

- **Problema.** Los tiradores son `<div>` controlados exclusivamente por
  eventos de puntero ([`src/App.tsx:851`](../src/App.tsx#L851)) y sus instrucciones
  sólo están en `title` ([`src/App.tsx:858`](../src/App.tsx#L858)).
- **Cambio propuesto.** Sustituirlos por separadores enfocables con
  `role="separator"`, valores ARIA y teclas de flecha/Home/End; mantener
  arrastrar y doble clic como atajos de puntero.
- **Aceptación.** Tab alcanza ambos separadores cuando están visibles; flechas
  cambian el ancho en incrementos predecibles, Home/End llegan a límites y el
  ancho persiste tras recargar.
- **Tamaño / prioridad.** M / P2.

## Orden sugerido

1. **U1** — elimina una barrera inmediata para abrir y gestionar historial de salas.
2. **U7** — evita la pérdida percibida de trabajo al fallar el envío de un prompt.
3. **U12** — hace utilizable por tecnologías de asistencia una vía central de navegación.

## Verificación de cambios de interfaz

La aceptación visual y de interacción requiere verificación humana en la app:

```bash
npm run tauri dev
```

Probar cada cambio en tema oscuro y claro, a 900×600, 1280×720 y 1440×900,
con teclado solamente. Para tareas de estados, usar respuestas de carga, vacío
y error controladas. Para accesibilidad, comprobar orden de Tab, foco visible y
anuncios con el lector de pantalla disponible en el sistema.

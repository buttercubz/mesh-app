// `prompts` namespace (WP-18, DESIGN §7.7): default spoken-prompt templates, one per
// mesh-api `PromptKey`, in i18next interpolation syntax. They are sent to the engine
// with `setPrompts` on every `startEngine` (engine/client.ts) and rendered to speech by
// the platform; prompts are short because riders hear them in a helmet.
//
// PLACEHOLDER (Appendix E item 9): the Demo-ride bot lines `demo.line1` … `demo.line8`
// are working placeholders until the user supplies the final lines.
import { defineCatalog } from "../index";

export const promptsCatalog = defineCatalog({
  en: {
    intercomLive: "Intercom live, {{n}} riders",
    memberJoined: "{{name}} joined",
    memberLeft: "{{name}} left",
    ridersOutOfRange: "{{n}} riders out of range",
    alone: "No riders in range",
    micPaused: "Mic paused. Tap the phone to resume",
    micBack: "Mic back",
    headsetLost: "Headset lost",
    bluetoothOff: "Bluetooth off",
    batteryLow: "Battery low",
    talkLimit: "Talk limit reached",
    voxLessSensitive: "Voice detection less sensitive",
    rideStarted: "Ride started",
    leaderChanged: "{{name}} is the leader now",
    "demo.line1": "Demo rider here. Can you hear me?",
    "demo.line2": "Turning left at the next junction.",
    "demo.line3": "Fuel stop in ten minutes.",
    "demo.line4": "Watch the gravel on this bend.",
    "demo.line5": "I'm at the back, all good.",
    "demo.line6": "Slowing down, traffic ahead.",
    "demo.line7": "Nice road. Keep this pace.",
    "demo.line8": "Demo ride over. Thanks for trying it.",
  },
  es: {
    intercomLive: "Intercomunicador activo, {{n}} motoristas",
    memberJoined: "{{name}} se unió",
    memberLeft: "{{name}} se fue",
    ridersOutOfRange: "{{n}} motoristas fuera de alcance",
    alone: "Nadie al alcance",
    micPaused: "Micrófono en pausa. Toque el teléfono para reanudar",
    micBack: "Micrófono de vuelta",
    headsetLost: "Casco desconectado",
    bluetoothOff: "Bluetooth apagado",
    batteryLow: "Batería baja",
    talkLimit: "Límite de habla alcanzado",
    voxLessSensitive: "Detección de voz menos sensible",
    rideStarted: "Rodada iniciada",
    leaderChanged: "{{name}} es ahora el líder",
    "demo.line1": "Aquí el motorista de prueba. ¿Me escucha?",
    "demo.line2": "Giro a la izquierda en el próximo cruce.",
    "demo.line3": "Parada para gasolina en diez minutos.",
    "demo.line4": "Cuidado con la gravilla en esta curva.",
    "demo.line5": "Voy atrás, todo bien.",
    "demo.line6": "Bajando la velocidad, tráfico adelante.",
    "demo.line7": "Buena carretera. Mantengamos este ritmo.",
    "demo.line8": "Fin de la rodada de prueba. Gracias por probarla.",
  },
});

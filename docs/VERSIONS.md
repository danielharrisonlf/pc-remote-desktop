# Versões verificadas em 15/10/2026

Os lockfiles são a referência exata de todas as dependências transitivas. As versões abaixo foram usadas nas builds locais que acompanham esta entrega; não são versões beta.

| Componente | Versão |
|---|---|
| Desktop / Android RN / Android Kotlin / protocolo | 0.2.2 / 0.2.1 / 0.2.4-kotlin / 1 + extensão opcional input.v1 |
| Node / npm | 24.21.0 / 11.19.0 |
| Android React Native / React | 0.87.1 / 19.2.3 |
| React Native Community CLI | 20.2.0 |
| Android TypeScript | 6.0.3 (resolução no lockfile) |
| Zod / Slider / Safe Area | 4.6.7 / 5.2.1 / 5.10.1 |
| OkHttp, resolução Gradle confirmada | 4.12.0 |
| Google Play Services Code Scanner | 16.1.0 |
| JDK / Gradle / Android Gradle Plugin | 17 / 9.4.1 / 9.2.1 |
| Android compile / target / mínimo | 37 / 36 / 24 |
| Pacote SDK / Build Tools / NDK | android-37.0 / 37.0.0 / 27.1.12297006 |
| Rust / MSVC / Windows SDK de build | 1.97.1 / 19.44 / 10.0.26100.0 |
| Tauri Rust / CLI / API JS | 2.12.3 / 2.12.2 / 2.12.3 |
| Tauri Single Instance | 2.5.3 |
| windows crate | 0.62.2 |
| Desktop React / TypeScript / Vite | 19.3.0 / 5.9.3 / 8.3.4 |
| Jest / Vitest | resolvidos no package-lock / 5.0.4 |

O Android utiliza New Architecture e Hermes, conforme o template oficial. A plataforma prioritária é Android e o template iOS não é versionado. Expo estável 57.0.28 acompanha RN 0.86; o Community CLI foi escolhido para utilizar RN estável atual 0.87.1. A build de release não depende do Expo Go ou Metro.

Target Android 36: o acesso LAN usa INTERNET e NSD. Ao elevar target para 37, implementar ACCESS_LOCAL_NETWORK com solicitação contextual ou o seletor autorizado do sistema, conforme [documentação Android](https://developer.android.com/privacy-and-security/local-network-permission). Não usar serviço em primeiro plano como alternativa à restrição de clipboard.

## Compatibilidade entre versões

| Android | Desktop | Compatibilidade |
|---|---|---|
| Kotlin 0.2.4-kotlin / React Native 0.2.1 | 0.2.2 | Protocolo 1; input.v1 após consentimento nos dois aplicativos |
| Kotlin 0.2.4-kotlin / React Native 0.2.1 | 0.2.0–0.2.1 | Mesmo contrato de volume, mídia, clipboard e input.v1 |
| Kotlin 0.2.0–0.2.3-kotlin / React Native 0.2.0 | 0.2.x | Compatibilidade anterior preservada; input.v1 após consentimento |
| Clientes Android 0.2.x | 0.1.x | Volume, mídia e clipboard; sem mouse/teclado |
| Qualquer implementação | Sem protocolo 1 | Incompatível; rejeitado |

A matriz descreve contrato; capacidades do dispositivo/player definem os controles reais. Não equivale a certificação de todos os players/hardwares.

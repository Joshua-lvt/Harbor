# Harbor Mobile

V0.1 Android observer for one Harbor PC. The app registers its own relay identity,
binds with a short-lived mobile code, and maintains a receive-only WebSocket in a
`connectedDevice` foreground service.

Configure the relay URL at build time:

```sh
./gradlew :app:assembleDebug -PharborRelayUrl=https://relay.example
```

The repository snapshot does not include a Gradle wrapper JAR. On a machine with
Java and Gradle installed, run `gradle wrapper` once, then use `./gradlew`.

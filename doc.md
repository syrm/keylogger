{
"sub": "client-id-123",
"htm": "POST",
"htu": "https://serveur/sync",
"body_hash": "sha256(body)",
"jti": "uuid-unique",
"iat": 1716800000,
"exp": 1716800060
}

Client → POST /sync
Authorization: <jwt signé ci-dessus>
body: { ...données... }

Serveur →

1. vérifie signature avec public_key en DB ← identité
2. vérifie jti jamais vu ← anti-replay
3. vérifie exp < 60s ← fenêtre courte
4. vérifie htm + htu == requête reçue ← bon endpoint
5. vérifie sha256(body reçu) == body_hash ← intégrité du body

on peut simplifier comme ça :

1. ENREGISTREMENT (une seule fois)
   Client → POST /auth/register (pre-register code, public key)
   Serveur → save la public key
2. ENVOI DES DONNÉES
   Client → crée un JWT signé avec sa clé privée :
   { sub: "client-id-123", iat: 1716800000, exp: 1716800060, // valide 60 secondes seulement jti: "uuid-unique" // évite
   le replay }
   Client → POST /sync { client_assertion: "<jwt signé>", header dpop: sha1 du body signé }
   Serveur → vérifie la signature avec la public_key en DB
   Serveur → vérifie que jti n'a pas déjà été utilisé (anti-replay)
   Serveur → vérifie que l'exp est courte (moins de 1h)
   Serveur → vérifie que le sha1 du body correspond au dpop et vérifie la signature du dpop
   Serveur → accepte les données de sync du post

Le DPoP header devrait aussi couvrir la méthode et l'URL, sinon le body hashé pourrait être rejoué sur un autre
endpoint :
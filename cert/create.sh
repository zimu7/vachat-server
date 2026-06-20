#!/bin/bash

# Create private key file
openssl genpkey -algorithm RSA \
    -pkeyopt rsa_keygen_bits:4096 \
    -pkeyopt rsa_keygen_pubexp:65537 | \
    openssl pkcs8 -topk8 -nocrypt -outform pem > vachat.com.key

# generate CSR file
openssl req -subj "/C=US/ST=Arizona/L=Scottsdale/O=vachat,Inc./CN=vachat.com/emailAddress=admin@vachat.com" \
    -new -days 3650 -key vachat.com.key -out vachat.com.csr

# generate self-sign file
openssl x509 -signkey vachat.com.key -in vachat.com.csr -req -days 365 -out vachat.com.crt

# view certificate
openssl req -text -noout -verify -in vachat.com.csr

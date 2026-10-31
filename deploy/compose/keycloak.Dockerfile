FROM quay.io/keycloak/keycloak:26.8.1@sha256:3f3c5f8e03c614774b4bd9173b88391fefa4153bfc010d8012d59ad825377332
RUN /opt/keycloak/bin/kc.sh build --db=postgres
USER 10001:10001
ENTRYPOINT ["/opt/keycloak/bin/kc.sh"]
CMD ["--config-file=/run/qgr/keycloak.conf", "start", "--optimized", "--import-realm"]

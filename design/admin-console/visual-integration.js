/* Compose the isolated design studies with the existing prototype interaction model. */
const priorOwnerDevices = ownerDevices;
const priorUsersView = usersView;
adminOverview = () => window.AdminVisual.overview();
telemetryView = () => window.TelemetryVisual.render();
ownerDevices = () => chosenDevice ? priorOwnerDevices() : window.UserVisual.render();
inbox = () => {
  const html = ownerDevices();
  return chosenDevice ? html : html.replace('<div class="uv-context">', '<div class="uv-context">' + approval());
};
usersView = () => chosenDevice || !chosenUser ? priorUsersView() : window.AdminVisual.userDetail(chosenUser);
render();

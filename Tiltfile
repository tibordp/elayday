k8s_yaml('kubernetes/manifest.yaml')

# Build: tell Tilt what images to build from which directories
docker_build('elayday', '.')

k8s_resource('elayday', port_forwards='8080:24602')
k8s_resource('elayday-reflector')
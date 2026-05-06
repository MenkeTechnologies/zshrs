echo $(echo hi)
echo "$(echo hi)"
echo $(echo $(echo nested))
echo "$(echo "$(echo double nested)")"

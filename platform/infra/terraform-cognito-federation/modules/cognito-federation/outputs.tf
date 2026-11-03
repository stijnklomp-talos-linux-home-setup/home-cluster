output "user_pool_id" {
  value = aws_cognito_user_pool.home.id
}

output "user_pool_issuer" {
  value = aws_cognito_user_pool.home.endpoint
}

output "pinniped_client_id" {
  value = aws_cognito_user_pool_client.pinniped.id
}

output "pinniped_client_secret" {
  value     = aws_cognito_user_pool_client.pinniped.client_secret
  sensitive = true
}

output "oauth2_proxy_client_id" {
  value = aws_cognito_user_pool_client.oauth2_proxy.id
}

output "oauth2_proxy_client_secret" {
  value     = aws_cognito_user_pool_client.oauth2_proxy.client_secret
  sensitive = true
}

output "argocd_client_id" {
  value = aws_cognito_user_pool_client.argocd.id
}

output "argocd_client_secret" {
  value     = aws_cognito_user_pool_client.argocd.client_secret
  sensitive = true
}
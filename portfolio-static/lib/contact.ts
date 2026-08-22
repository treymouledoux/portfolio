export interface ContactFormData {
    name: string
    email: string
    message: string
}

export interface ContactResponse {
    success: boolean
    reply_message?: string
}

export async function submitContactForm(data: ContactFormData): Promise<ContactResponse> {
    const response = await fetch('/api/contact', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    })

    const contentType = response.headers.get('content-type') ?? ''
    if (!contentType.includes('application/json')) {
        throw new Error(`Contact endpoint returned ${response.status} instead of JSON`)
    }

    const result = await response.json() as ContactResponse
    if (!response.ok) {
        throw new Error(result.reply_message ?? 'Unable to send message')
    }

    return result
}
